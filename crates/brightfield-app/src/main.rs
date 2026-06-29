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
use brightfield_render::scene::{build_chart_scene, build_multi_mark_scene, ChartData};
use brightfield_spec::analysis::analyse_spec;
use brightfield_spec::parse_spec_path;
use brightfield_spec::vocab::MarkKind;
use brightfield_sql::collect_marks;

use brightfield_render::ScaleSet;

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

/// Run the spec-to-scene pipeline, returning the scene, scales and mark count.
///
/// Returns `Err` (rather than exiting) on any failure, so callers can recover —
/// the hot-reload watcher keeps the last good chart when a mid-edit save is
/// momentarily invalid. `main` turns the initial error into a clean exit.
fn run_pipeline(spec_path: &str) -> Result<(vello::Scene, ScaleSet, usize), String> {
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

    // 4. Execute all marks, collecting successful results (AC-05: graceful failure).
    let results = session.execute_all();
    let marks = collect_marks(&parsed.spec);

    let mut chart_entries: Vec<(arrow::record_batch::RecordBatch, ChannelMap, MarkKind)> =
        Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(batches) => {
                // DuckDB streams results one batch per ~2048-row vector, so a
                // query wider than a single chunk arrives as several batches.
                // Concatenate them; keeping only the first silently truncates
                // the result (a large scatter would render only its first chunk).
                if let Some(batch) = concat_result_batches(batches) {
                    let mark = marks[i];
                    let cm = ChannelMap::from_mark(mark);
                    chart_entries.push((batch, cm, mark.kind));
                }
            }
            Err(e) => {
                eprintln!("warning: skipping mark {i}: {e}");
            }
        }
    }

    if chart_entries.is_empty() {
        return Err("no marks rendered successfully".to_string());
    }

    // 5. Build the scene.
    let width = 640.0_f64;
    let height = 480.0_f64;
    let layout = ChartLayout::new(width, height);

    // Build the default renderer registry once and dispatch per-mark via
    // find_renderer. Marks whose kind has no registered renderer are skipped
    // with a warning (no silent dot fallback).
    let registry = default_renderers();
    let chart_data: Vec<ChartData<'_>> = chart_entries
        .iter()
        .filter_map(|(batch, cm, kind)| {
            let renderer = match find_renderer(&registry, *kind) {
                Some(r) => r,
                None => {
                    eprintln!("warning: no renderer for mark kind {kind:?} — skipping");
                    return None;
                }
            };
            Some(ChartData {
                batch,
                channel_map: cm,
                renderer,
                layout: layout.clone(),
                view_extent: None,
                highlight: None,
            })
        })
        .collect();

    let mark_count = chart_data.len();
    let (scene, scales) = if chart_data.len() == 1 {
        build_chart_scene(&chart_data[0])
    } else {
        let refs: Vec<&ChartData<'_>> = chart_data.iter().collect();
        build_multi_mark_scene(&refs)
    };

    Ok((scene, scales, mark_count))
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
#[cfg(target_os = "macos")]
fn spawn_spec_watcher(
    cx: &mut gpui::App,
    state: gpui::Entity<brightfield_ui::ChartState>,
    spec_path: String,
) {
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

            // Re-run the (blocking) pipeline off the main thread; keep only the
            // scene (Scene is Send) so the result can cross the thread boundary.
            // catch_unwind contains a panicking pipeline (a parses-but-degenerate
            // mid-edit spec) so a bad save keeps the last good chart rather than
            // crashing the window — the same guarantee the Err paths give.
            let path = spec_path.clone();
            let built = cx
                .background_executor()
                .spawn(async move {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_pipeline(&path)))
                        .unwrap_or_else(|_| Err("pipeline panicked".to_string()))
                        .map(|(scene, _, _)| scene)
                })
                .await;

            match built {
                Ok(scene) => {
                    // Swap in the new scene and repaint, flushed in one update cycle.
                    cx.update(|app| {
                        state.update(app, |s, c| {
                            s.set_scene(scene);
                            c.notify();
                        });
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

    let (scene, scales, mark_count) = match run_pipeline(spec_path) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    eprintln!(
        "Pipeline complete: {mark_count} mark(s) rendered, {} scale(s) inferred",
        ["x", "y"]
            .iter()
            .filter(|ch| scales
                .get(brightfield_render::channel::Channel::from_wire(ch).unwrap())
                .is_some())
            .count()
    );

    // Print scene stats for verification.
    let encoding = scene.encoding();
    eprintln!(
        "Scene: {} path tags, {} draw tags",
        encoding.path_tags.len(),
        encoding.draw_tags.len()
    );

    // Debug path: dump rendered output to a PNG instead of opening a window.
    // Triggered by `BRIGHTFIELD_DUMP_PNG=<path> brightfield <spec.yaml>`.
    if let Ok(dump_path) = env::var("BRIGHTFIELD_DUMP_PNG") {
        // Optional supersampling for HiDPI verification: BRIGHTFIELD_DUMP_SCALE=2
        // renders at device resolution via the same scale-the-scene path the
        // window uses for crisp Retina output.
        let scale: f32 = env::var("BRIGHTFIELD_DUMP_SCALE")
            .ok()
            .and_then(|s| s.parse().ok())
            .filter(|s: &f32| *s > 0.0)
            .unwrap_or(1.0);
        let dev_w = (640.0 * scale).round() as u32;
        let dev_h = (480.0 * scale).round() as u32;
        let mut scaled = vello::Scene::new();
        scaled.append(&scene, Some(vello::kurbo::Affine::scale(f64::from(scale))));

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
        let _ = scales;
        return;
    }

    // Open a native GPUI window and display the rendered scene.
    #[cfg(target_os = "macos")]
    {
        use std::rc::Rc;
        use gpui::AppContext;

        let renderer = brightfield_ui::VelloRenderer::new();
        let app = gpui::Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)));
        let _ = scales; // scales currently unused by the window path
        let spec_path = spec_path.to_string();
        app.run(move |cx| {
            let state = cx.new(|_| {
                brightfield_ui::ChartState::new(scene, 640, 480, renderer)
            });
            let view_state = state.clone();
            let _window = cx
                .open_window(gpui::WindowOptions::default(), move |_window, cx| {
                    cx.new(|_| brightfield_ui::ChartView::new(view_state))
                })
                .expect("failed to open window");

            // Hot-reload: swap in a freshly rendered scene when the spec changes.
            spawn_spec_watcher(cx, state, spec_path);
        });
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = scene;
        let _ = scales;
        eprintln!("GPUI window display is currently macOS-only.");
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
