//! Brightfield application entry point.
//!
//! Orchestrates the full spec-to-chart pipeline:
//! parse → analyse → engine → execute → render → display.
//!
//! The GPUI window requires a platform implementation (gpui_macos on macOS,
//! which needs full Xcode + Metal compiler). Without it, the pipeline runs
//! headlessly and prints a summary.

use std::env;
use std::path::Path;
use std::process;

use brightfield_engine::Engine;
use brightfield_render::channel::ChannelMap;
use brightfield_render::layout::ChartLayout;
use brightfield_render::mark::{default_renderers, find_renderer};
use brightfield_render::scene::{build_multi_mark_scene, compose_dashboard, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::layout::{placed_plots, Rect};
use brightfield_spec::parse_spec_path;
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::{collect_marks, collect_plot_groups};

/// Concatenate the record batches from one mark's query into a single batch.
///
/// DuckDB streams results one batch per internal vector (~2048 rows), so a
/// query returning more than one chunk arrives as several batches. Rendering
/// only the first would silently drop the rest. Returns `None` for an empty
/// result. On the rare concat failure, falls back to the first batch (with a
/// warning) rather than dropping the mark entirely.
fn concat_result_batches(
    batches: Vec<arrow::record_batch::RecordBatch>,
) -> Option<arrow::record_batch::RecordBatch> {
    match batches.len() {
        0 => None,
        1 => batches.into_iter().next(),
        _ => {
            let schema = batches[0].schema();
            match arrow::compute::concat_batches(&schema, &batches) {
                Ok(batch) => Some(batch),
                Err(e) => {
                    eprintln!(
                        "warning: could not concatenate {} result batches ({e}); \
                         rendering the first chunk only",
                        batches.len()
                    );
                    batches.into_iter().next()
                }
            }
        }
    }
}

/// One rendered plot: its component-path identity, position/size in the
/// dashboard, and its own scene.
struct PlotRender {
    path: String,
    x: f64,
    y: f64,
    width: u32,
    height: u32,
    scene: vello::Scene,
}

/// A rendered dashboard: the bounding-box dimensions and one scene per plot.
/// The headless/PNG path composites these; the window hosts one element per plot.
struct Dashboard {
    width: u32,
    height: u32,
    plots: Vec<PlotRender>,
}

/// Run the spec-to-scene pipeline, returning a [`Dashboard`] — one independently
/// rendered scene per plot (each with its own axes/scales), positioned per the
/// layout pass. A single plot is just a one-plot dashboard.
///
/// Returns `Err` (rather than exiting) on any failure, so callers can recover —
/// the hot-reload watcher keeps the last good chart when a mid-edit save is
/// momentarily invalid. `main` turns the initial error into a clean exit.
fn run_pipeline(spec_path: &str) -> Result<Dashboard, String> {
    // 1. Parse the spec.
    let parsed = parse_spec_path(spec_path).map_err(|e| format!("parse error: {e}"))?;
    for w in &parsed.warnings {
        eprintln!("parse warning: {w:?}");
    }

    // 2. Analyse the spec.
    let analysis = analyse_spec(&parsed.spec).map_err(|e| format!("analysis error: {e}"))?;

    // 3. Load into engine (creates DuckDB views).
    let engine = Engine::new();
    let spec_dir = Path::new(spec_path).parent();
    let load = engine
        .load_spec(parsed.spec.clone(), analysis, spec_dir)
        .map_err(|e| format!("engine error: {e}"))?;
    let mut session = load.session;

    // 4. Execute all marks, building per-mark inputs indexed by the flat mark
    //    order (= execution order). A failed mark becomes None and is skipped
    //    (AC-05: graceful failure). Batches are concatenated so a >2048-row
    //    result isn't silently truncated to its first chunk.
    let results = session.execute_all();
    let marks = collect_marks(&parsed.spec);
    let mut mark_inputs: Vec<Option<(arrow::record_batch::RecordBatch, ChannelMap, MarkKind)>> =
        Vec::with_capacity(marks.len());
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(batches) => match concat_result_batches(batches) {
                Some(batch) => {
                    let mark = marks[i];
                    mark_inputs.push(Some((batch, ChannelMap::from_mark(mark), mark.kind)));
                }
                None => mark_inputs.push(None),
            },
            Err(e) => {
                eprintln!("warning: skipping mark {i}: {e}");
                mark_inputs.push(None);
            }
        }
    }

    // 5. Lay the plots out, group each plot's marks, and build one scene per
    //    plot (its own axes/scales) at the position from the layout pass.
    let placed = placed_plots(&parsed.spec, Rect::new(0.0, 0.0, 0.0, 0.0));
    let groups = collect_plot_groups(&parsed.spec);
    let registry = default_renderers();

    let mut plots: Vec<PlotRender> = Vec::new();
    for plot in &placed {
        let group = match groups.iter().find(|g| g.plot_path == plot.path) {
            Some(g) => g,
            None => continue,
        };
        let layout = ChartLayout::new(plot.rect.width, plot.rect.height);
        let chart_data: Vec<ChartData<'_>> = group
            .mark_indices
            .iter()
            .filter_map(|&mi| {
                let (batch, cm, kind) = mark_inputs[mi].as_ref()?;
                match find_renderer(&registry, *kind) {
                    Some(renderer) => Some(ChartData {
                        batch,
                        channel_map: cm,
                        renderer,
                        layout: layout.clone(),
                        view_extent: None,
                        highlight: None,
                    }),
                    None => {
                        eprintln!("warning: no renderer for mark kind {kind:?} — skipping");
                        None
                    }
                }
            })
            .collect();
        if chart_data.is_empty() {
            continue;
        }
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let (scene, _scales) = build_multi_mark_scene(&refs);
        plots.push(PlotRender {
            path: plot.path.clone(),
            x: plot.rect.x,
            y: plot.rect.y,
            width: plot.rect.width.ceil() as u32,
            height: plot.rect.height.ceil() as u32,
            scene,
        });
    }

    if plots.is_empty() {
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
    Ok(Dashboard { width, height, plots })
}

/// Last-modified time of the spec file, for change detection. `None` if the
/// file is momentarily unreadable (e.g. mid-save), which the watcher treats as
/// a change and retries.
#[cfg(target_os = "macos")]
fn file_mtime(path: &str) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// Spawn a background task that polls the spec file and swaps in a freshly
/// rendered scene when it changes — turning the edit→see loop interactive
/// without a restart. A save that fails to parse/execute keeps the last good
/// chart (the warning is printed) rather than killing the window.
/// A plot the watcher tracks: its stable component path, fixed geometry, and
/// reactive state. Hot-reload matches new plots to these by `path`.
#[cfg(target_os = "macos")]
struct WatchedPlot {
    path: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    state: gpui::Entity<brightfield_ui::ChartState>,
}

#[cfg(target_os = "macos")]
fn spawn_spec_watcher(cx: &mut gpui::App, watched: Vec<WatchedPlot>, spec_path: String) {
    const POLL: std::time::Duration = std::time::Duration::from_millis(300);

    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        let mut last = file_mtime(&spec_path);
        loop {
            cx.background_executor().timer(POLL).await;
            let now = file_mtime(&spec_path);
            if now == last {
                continue;
            }
            last = now;

            // Re-run the (blocking) pipeline off the main thread (Dashboard is
            // Send). catch_unwind contains a panicking pipeline (a degenerate
            // mid-edit spec) so a bad save keeps the last good chart rather than
            // crashing the window — the same guarantee the Err paths give.
            let path = spec_path.clone();
            let built = cx
                .background_executor()
                .spawn(async move {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_pipeline(&path)))
                        .unwrap_or_else(|_| Err("pipeline panicked".to_string()))
                })
                .await;

            match built {
                Ok(dashboard) => {
                    // Only a data/visual change is hot-swappable: the window's
                    // plot layout is fixed at launch, so require exactly the same
                    // plots (by stable path) at the same geometry. Any structural
                    // change (count, which plots render, or any size/position)
                    // can't be absorbed in place — restart to apply.
                    let same_layout = dashboard.plots.len() == watched.len()
                        && dashboard.plots.iter().all(|p| {
                            watched.iter().any(|w| {
                                w.path == p.path
                                    && w.x == p.x
                                    && w.y == p.y
                                    && w.width == f64::from(p.width)
                                    && w.height == f64::from(p.height)
                            })
                        });
                    if !same_layout {
                        eprintln!("reload skipped: dashboard layout changed; restart to apply");
                        continue;
                    }
                    // Swap each plot's new scene into its state (matched by path),
                    // then repaint once.
                    let mut scenes: std::collections::HashMap<String, vello::Scene> =
                        dashboard.plots.into_iter().map(|p| (p.path, p.scene)).collect();
                    cx.update(|app| {
                        for w in &watched {
                            if let Some(scene) = scenes.remove(&w.path) {
                                w.state.update(app, |s, c| {
                                    s.set_scene(scene);
                                    c.notify();
                                });
                            }
                        }
                        app.refresh_windows();
                    });
                    eprintln!("reloaded {spec_path}");
                }
                Err(e) => {
                    eprintln!("reload skipped (keeping last good chart): {e}");
                }
            }
        }
    })
    .detach();
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: brightfield <spec.yaml>");
        process::exit(1);
    }
    let spec_path = &args[1];

    let dashboard = match run_pipeline(spec_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    eprintln!(
        "Pipeline complete: {}x{} dashboard, {} plot(s)",
        dashboard.width,
        dashboard.height,
        dashboard.plots.len()
    );

    // Debug path: composite the per-plot scenes and dump a PNG instead of
    // opening a window. Triggered by `BRIGHTFIELD_DUMP_PNG=<path>`.
    if let Ok(dump_path) = env::var("BRIGHTFIELD_DUMP_PNG") {
        // Optional supersampling for HiDPI verification: BRIGHTFIELD_DUMP_SCALE=2
        // renders at device resolution via the same scale-the-scene path the
        // window uses for crisp Retina output.
        let scale: f32 = env::var("BRIGHTFIELD_DUMP_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|s: &f32| *s > 0.0)
            .unwrap_or(1.0);
        let placements: Vec<(f64, f64, &vello::Scene)> =
            dashboard.plots.iter().map(|p| (p.x, p.y, &p.scene)).collect();
        let composite = compose_dashboard(
            f64::from(dashboard.width),
            f64::from(dashboard.height),
            &placements,
        );

        let dev_w = ((dashboard.width as f32) * scale).round() as u32;
        let dev_h = ((dashboard.height as f32) * scale).round() as u32;
        let mut scaled = vello::Scene::new();
        scaled.append(&composite, Some(vello::kurbo::Affine::scale(f64::from(scale))));

        let renderer = brightfield_ui::VelloRenderer::new();
        let pixels = renderer
            .lock()
            .expect("renderer mutex poisoned")
            .render_to_pixels(&scaled, dev_w, dev_h);
        let img = image::RgbaImage::from_raw(dev_w, dev_h, pixels)
            .expect("pixel buffer size mismatch");
        img.save(&dump_path).expect("failed to write PNG");
        let non_zero = img.as_raw().iter().filter(|&&b| b != 0).count();
        let total = img.as_raw().len();
        eprintln!(
            "PNG dumped: {dump_path} ({dev_w}x{dev_h}, {non_zero}/{total} non-zero bytes, {:.1}% coverage)",
            100.0 * non_zero as f64 / total as f64
        );
        return;
    }

    // Open a native GPUI window: one ChartElement per plot, positioned per the
    // layout, each with its own ChartState (so interaction is per-plot).
    #[cfg(target_os = "macos")]
    {
        use gpui::AppContext;
        use std::rc::Rc;

        let renderer = brightfield_ui::VelloRenderer::new();
        let app = gpui::Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)));
        let spec_path = spec_path.to_string();
        let Dashboard { width, height, plots } = dashboard;
        app.run(move |cx| {
            // One ChartState (and element) per plot; the watcher tracks each by
            // its stable path + geometry for hot-reload.
            let mut charts: Vec<brightfield_ui::PlacedChart> = Vec::with_capacity(plots.len());
            let mut watched: Vec<WatchedPlot> = Vec::with_capacity(plots.len());
            for p in plots {
                let (x, y, w, h) = (p.x, p.y, f64::from(p.width), f64::from(p.height));
                let state = cx.new(|_| {
                    brightfield_ui::ChartState::new(p.scene, p.width, p.height, renderer.clone())
                });
                watched.push(WatchedPlot {
                    path: p.path,
                    x,
                    y,
                    width: w,
                    height: h,
                    state: state.clone(),
                });
                charts.push(brightfield_ui::PlacedChart {
                    x,
                    y,
                    width: w,
                    height: h,
                    state,
                });
            }

            let _window = cx
                .open_window(gpui::WindowOptions::default(), move |_window, cx| {
                    cx.new(|_| {
                        brightfield_ui::ChartView::new(f64::from(width), f64::from(height), charts)
                    })
                })
                .expect("failed to open window");

            // Hot-reload: swap each plot's scene when the spec changes on disk.
            spawn_spec_watcher(cx, watched, spec_path);
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = dashboard;
        eprintln!(
            "GPUI window display is currently macOS-only. \
             Re-run with BRIGHTFIELD_DUMP_PNG=out.png to render the chart to an image."
        );
    }
}

#[cfg(test)]
mod tests {
    use brightfield_engine::Engine;
    use brightfield_render::channel::{Channel, ChannelMap};
    use brightfield_render::layout::ChartLayout;
    use brightfield_render::mark::{DotRenderer, MarkRenderer};
    use brightfield_render::scene::{build_multi_mark_scene, ChartData};
    use brightfield_spec::analysis::analyse_spec;
    use brightfield_spec::{parse_spec, Format};

    #[test]
    fn concat_result_batches_combines_chunks_not_truncates() {
        use arrow::array::Int32Array;
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use std::sync::Arc;

        let schema = Arc::new(Schema::new(vec![Field::new("x", DataType::Int32, false)]));
        let chunk1 =
            RecordBatch::try_new(schema.clone(), vec![Arc::new(Int32Array::from(vec![1, 2]))])
                .unwrap();
        let chunk2 =
            RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![3, 4, 5]))]).unwrap();

        // Empty result -> None (nothing to render).
        assert!(super::concat_result_batches(vec![]).is_none());

        // Single chunk -> passed through unchanged.
        let single = super::concat_result_batches(vec![chunk1.clone()]).unwrap();
        assert_eq!(single.num_rows(), 2);

        // Multiple chunks -> concatenated (2 + 3 = 5), NOT truncated to the first.
        let combined = super::concat_result_batches(vec![chunk1, chunk2]).unwrap();
        assert_eq!(combined.num_rows(), 5, "all chunks must be retained, not just the first");
    }

    #[test]
    fn run_pipeline_returns_err_on_bad_spec_instead_of_exiting() {
        // The pipeline must return Err (not process::exit) so the hot-reload
        // watcher can keep the last good chart when a save is momentarily bad.
        // If run_pipeline still exited, this test process would die here.
        let missing = super::run_pipeline("/nonexistent/brightfield/spec.yaml");
        assert!(missing.is_err(), "missing spec should return Err, not exit");
    }

    #[test]
    fn msv_ac05_graceful_failure_skips_invalid_mark() {
        // Spec with one valid mark (dot, data.from) and one invalid (rect, unsupported).
        let yaml = r#"
data:
  t:
    - { x: 1, y: 10 }
    - { x: 2, y: 20 }
plot:
  - mark: dot
    data: { from: t }
    x: x
    y: y
  - mark: rect
    data: { from: t }
"#;
        let parsed = parse_spec(yaml, Format::Yaml).expect("parse failed");
        let analysis = analyse_spec(&parsed.spec).expect("analysis failed");

        let engine = Engine::new();
        let load = engine
            .load_spec(parsed.spec.clone(), analysis, None)
            .expect("load_spec failed");
        let mut session = load.session;

        // Execute all marks — dot should succeed, rect should fail.
        let results = session.execute_all();

        let mut successful = Vec::new();
        let mut skipped = 0_usize;
        for (i, result) in results.into_iter().enumerate() {
            match result {
                Ok(batches) => {
                    if let Some(batch) = batches.into_iter().next() {
                        successful.push((i, batch));
                    }
                }
                Err(_) => {
                    skipped += 1;
                }
            }
        }

        // Exactly one mark skipped (rect), one succeeded (dot).
        assert_eq!(skipped, 1, "expected 1 skipped mark");
        assert_eq!(successful.len(), 1, "expected 1 successful mark");

        // The successful batch should have the inline data (2 rows).
        let (_, batch) = &successful[0];
        assert_eq!(batch.num_rows(), 2);
        assert!(batch.schema().index_of("x").is_ok());
        assert!(batch.schema().index_of("y").is_ok());

        // Build scene from the valid mark — manually construct channel map
        // since we're testing the pipeline's graceful failure, not from_mark.
        let mut cm = ChannelMap::new();
        cm.insert(Channel::X, "x".to_string());
        cm.insert(Channel::Y, "y".to_string());

        let layout = ChartLayout::new(640.0, 480.0);
        let renderer: Box<dyn MarkRenderer> = Box::new(DotRenderer);

        let chart_data = vec![ChartData {
            batch,
            channel_map: &cm,
            renderer: renderer.as_ref(),
            layout,
            view_extent: None,
            highlight: None,
        }];
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        let (scene, scales) = build_multi_mark_scene(&refs);

        // Scene should be non-empty (the valid dot mark rendered).
        let encoding = scene.encoding();
        assert!(
            encoding.path_tags.len() > 0,
            "scene should have content from the valid mark"
        );

        // Scales should exist for x and y from the dot mark.
        assert!(scales.get(Channel::X).is_some(), "x scale should exist");
        assert!(scales.get(Channel::Y).is_some(), "y scale should exist");
    }
}
