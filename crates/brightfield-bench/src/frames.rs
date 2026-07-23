//! The shell-level suites: headless full-window frame times through
//! `brightfield_shell::capture::bench_frames` — the same real `MeridianApp`
//! the live window runs, rendered by egui's real wgpu backend.

use std::path::Path;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::bench_frames;
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_spec::analysis::ComponentPath;
use serde::Serialize;

use crate::scenario::brush_select;
use crate::stats::Stats;

/// Frame-time measurements for one booted spec.
#[derive(Debug, Clone, Serialize)]
pub struct FrameMeasurement {
    /// Steady-state frames: the app draws with nothing changing — the shell's
    /// floor (egui pass + composite of the cached canvas texture + GPU wait).
    pub steady: Stats,
    /// Interaction frames: every frame pushes one committed brush step through
    /// the live document before drawing, so each timed frame carries the
    /// re-query, the re-composite, the canvas re-raster and the GPU wait —
    /// the true in-frame cost of a brush step in the live window. `None` for
    /// corpus specs, which are measured steady-state only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interaction: Option<Stats>,
}

/// Boot `spec_path` and time `measured` steady-state frames after `warmup`
/// discarded frames.
pub fn frames_steady(
    spec_path: &Path,
    scale: f32,
    warmup: usize,
    measured: usize,
) -> Result<Stats, String> {
    let boot = Boot::open(
        spec_path.to_str().ok_or("spec path is not UTF-8")?,
        Flow::default(),
        None,
    )?;
    let times = bench_frames(boot, Mode::Light, scale, warmup + measured, |_, _| {})?;
    Stats::from_durations(&times[warmup..]).ok_or_else(|| "no frames measured".to_string())
}

/// Boot `spec_path` live and time frames that each carry one brush step over
/// `brush_column` within `brush_domain`.
///
/// Fails rather than reports if the boot has no live session — an interaction
/// frame against a still document would time nothing but the draw.
#[allow(clippy::too_many_arguments)]
pub fn frames_interaction(
    spec_path: &Path,
    brush_column: &str,
    brush_domain: (f64, f64),
    selection: &str,
    contributor: &ComponentPath,
    scale: f32,
    warmup: usize,
    measured: usize,
) -> Result<Stats, String> {
    let boot = Boot::open(
        spec_path.to_str().ok_or("spec path is not UTF-8")?,
        Flow::default(),
        None,
    )?;
    let selection = selection.to_string();
    let contributor = contributor.clone();
    let brush_column = brush_column.to_string();
    let mut applied = 0usize;
    let times = bench_frames(boot, Mode::Light, scale, warmup + measured, |app, i| {
        if app.chart_doc_mut().apply_interaction(brush_select(
            &brush_column,
            brush_domain,
            &selection,
            &contributor,
            i,
        )) {
            applied += 1;
        }
    })?;
    if applied != warmup + measured {
        return Err(format!(
            "interaction frames: only {applied}/{} applies landed — the document was not live",
            warmup + measured
        ));
    }
    Stats::from_durations(&times[warmup..]).ok_or_else(|| "no frames measured".to_string())
}
