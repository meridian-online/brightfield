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

/// Run the spec-to-scene pipeline, returning the scene and entry count.
fn run_pipeline(
    spec_path: &str,
) -> (
    vello::Scene,
    ScaleSet,
    usize, // mark count
) {
    // 1. Parse the spec.
    let parsed = match parse_spec_path(spec_path) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Parse error: {e}");
            process::exit(1);
        }
    };
    for w in &parsed.warnings {
        eprintln!("parse warning: {w:?}");
    }

    // 2. Analyse the spec.
    let analysis = match analyse_spec(&parsed.spec) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("Analysis error: {e}");
            process::exit(1);
        }
    };

    // 3. Load into engine (creates DuckDB views).
    let engine = Engine::new();
    let spec_dir = Path::new(spec_path).parent();
    let load = match engine.load_spec(parsed.spec.clone(), analysis, spec_dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Engine error: {e}");
            process::exit(1);
        }
    };
    let mut session = load.session;

    // 4. Execute all marks, collecting successful results (AC-05: graceful failure).
    let results = session.execute_all();
    let marks = collect_marks(&parsed.spec);

    let mut chart_entries: Vec<(arrow::record_batch::RecordBatch, ChannelMap, MarkKind)> =
        Vec::new();
    for (i, result) in results.into_iter().enumerate() {
        match result {
            Ok(batches) => {
                if let Some(batch) = batches.into_iter().next() {
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
        eprintln!("No marks rendered successfully.");
        process::exit(1);
    }

    // 5. Build the scene.
    let width = 640.0_f64;
    let height = 480.0_f64;
    let layout = ChartLayout::new(width, height);

    // Build the default renderer registry once and dispatch per-mark via
    // find_renderer. Marks whose kind has no registered renderer are skipped
    // with a tracing event (no silent dot fallback).
    let registry = default_renderers();
    let chart_data: Vec<ChartData<'_>> = chart_entries
        .iter()
        .filter_map(|(batch, cm, kind)| {
            let renderer = match find_renderer(&registry, *kind) {
                Some(r) => r,
                None => {
                    tracing::warn!(
                        mark = ?kind,
                        "no renderer registered for mark kind — skipping"
                    );
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

    (scene, scales, mark_count)
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: brightfield <spec.yaml>");
        process::exit(1);
    }
    let spec_path = &args[1];

    let (scene, scales, mark_count) = run_pipeline(spec_path);

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
        let renderer = brightfield_ui::VelloRenderer::new();
        let pixels = renderer
            .lock()
            .expect("renderer mutex poisoned")
            .render_to_pixels(&scene, 640, 480);
        let img = image::RgbaImage::from_raw(640, 480, pixels)
            .expect("pixel buffer size mismatch");
        img.save(&dump_path).expect("failed to write PNG");
        let non_zero = img.as_raw().iter().filter(|&&b| b != 0).count();
        let total = img.as_raw().len();
        eprintln!(
            "PNG dumped: {dump_path} ({non_zero}/{total} non-zero bytes, {:.1}% coverage)",
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
        app.run(move |cx| {
            let state = cx.new(|_| {
                brightfield_ui::ChartState::new(scene, 640, 480, renderer)
            });
            let _window = cx
                .open_window(gpui::WindowOptions::default(), |_window, cx| {
                    cx.new(|_| brightfield_ui::ChartView::new(state))
                })
                .expect("failed to open window");
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
