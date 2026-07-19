//! `brightfield-shot` — headless real-UI capture (Tier-2 of the loop).
//!
//! Boots the real egui shell over a spec, renders the whole window through
//! egui_wgpu into an offscreen texture, and writes a PNG — no display required.
//! This is the capability the gpui host never had: an agent edits the UI and
//! sees the actual pixels, including scripted keystrokes driving the view.
//!
//! Two surfaces, auto-routed by the spec:
//! - a **Mosaic dashboard** (`--spec dashboard.yaml`) → the chart shell;
//! - a **Protocol manifest** (`arcform.yaml`, with `BRIGHTFIELD_PROTOCOL_OFFLINE=1`)
//!   → the egui Protocol panel (dock + DAG + outline + inspector + steps).
//!
//! Usage:
//!   brightfield-shot --spec S.yaml --out out.png
//!                    [--size WxH] [--scale N] [--theme light|dark]
//!                    [--script keys.ndjson]
//!                    [--flow vertical|horizontal] [--focus <dotted-id>]
//!
//! `--flow` / `--focus` apply to the Protocol panel: the reading axis (vertical
//! by default) and the boot selection (the dotted asset id a click would target,
//! so a scripted `za`/`Enter` has a cursor to act on).

use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::{
    capture_png, capture_protocol_png, capture_vello_only, parse_script,
};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;
use brightfield_shell::protocol::load_protocol_offline;

struct Args {
    spec: String,
    out: PathBuf,
    scale: f32,
    mode: Mode,
    script: Option<PathBuf>,
    size: Option<(u32, u32)>,
    vello_only: bool,
    flow: Flow,
    focus: Option<String>,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: brightfield-shot --spec S.yaml --out O.png \
                 [--scale N] [--theme light|dark] [--script keys.ndjson] \
                 [--size WxH] [--flow vertical|horizontal] [--focus <dotted-id>]"
            );
            return ExitCode::from(2);
        }
    };

    let script = match &args.script {
        Some(p) => match parse_script(p) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("script error: {e}");
                return ExitCode::from(1);
            }
        },
        None => Vec::new(),
    };

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    // Route a Protocol manifest to the egui Protocol panel.
    let text = std::fs::read_to_string(&args.spec).unwrap_or_default();
    if brightfield_protocol::is_protocol_manifest(&text) {
        return run_protocol(&args, script);
    }

    // Otherwise the Mosaic dashboard path.
    let composed = match compose_spec(&args.spec) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("pipeline error: {e}");
            return ExitCode::from(1);
        }
    };
    eprintln!(
        "composed {}x{} dashboard from {}",
        composed.width, composed.height, args.spec
    );

    let _ = args.size; // reserved override; the shell auto-sizes today.
    let result = if args.vello_only {
        capture_vello_only(composed, args.scale, &args.out)
    } else {
        capture_png(composed, args.mode, args.scale, &args.out, script)
    };
    report(result, &args.out)
}

/// Build the Protocol inputs (gated offline) and capture the panel.
fn run_protocol(args: &Args, script: Vec<Vec<egui::Event>>) -> ExitCode {
    if std::env::var("BRIGHTFIELD_PROTOCOL_OFFLINE").is_err() {
        eprintln!(
            "{} is a Protocol manifest, not an emitted Protocol+Run contract. \
             To render it offline without a run, set BRIGHTFIELD_PROTOCOL_OFFLINE=1.",
            args.spec
        );
        return ExitCode::from(1);
    }
    let inputs = match load_protocol_offline(&args.spec) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("protocol pipeline error: {e}");
            return ExitCode::from(1);
        }
    };
    eprintln!(
        "protocol {} ({} collapsed / {} full nodes, {} steps, {:?} flow) from {}",
        inputs.protocol,
        inputs.graph_collapsed.nodes.len(),
        inputs.graph_full.nodes.len(),
        inputs.sheet_rows.len(),
        args.flow,
        args.spec
    );
    // Surface the family tile ids so `--focus` has a target for a scripted `za`.
    for (id, node) in &inputs.graph_collapsed.nodes {
        if node.kind == brightfield_protocol::graph::AssetKind::Family {
            eprintln!("  family tile: {id}");
        }
    }
    let result = capture_protocol_png(
        inputs,
        args.mode,
        args.flow,
        args.focus.clone(),
        args.scale,
        &args.out,
        script,
    );
    report(result, &args.out)
}

fn report(result: Result<(u32, u32), String>, out: &std::path::Path) -> ExitCode {
    match result {
        Ok((w, h)) => {
            eprintln!("wrote {} ({w}x{h} device px)", out.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("capture error: {e}");
            ExitCode::from(1)
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut spec: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut scale = 2.0_f32;
    let mut mode = Mode::Light;
    let mut script = None;
    let mut size = None;
    let mut vello_only = false;
    let mut flow = Flow::Vertical;
    let mut focus = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--spec" => spec = Some(next()?),
            "--out" => out = Some(PathBuf::from(next()?)),
            "--scale" => scale = next()?.parse().map_err(|_| "--scale not a number".to_string())?,
            "--theme" => mode = if next()? == "dark" { Mode::Dark } else { Mode::Light },
            "--vello-only" => vello_only = true,
            "--script" => script = Some(PathBuf::from(next()?)),
            "--flow" => {
                flow = match next()?.as_str() {
                    "horizontal" | "h" => Flow::Horizontal,
                    "vertical" | "v" => Flow::Vertical,
                    other => return Err(format!("--flow vertical|horizontal, not {other}")),
                }
            }
            "--focus" => focus = Some(next()?),
            "--size" => {
                let v = next()?;
                let (w, h) = v.split_once('x').ok_or_else(|| "--size WxH".to_string())?;
                size = Some((
                    w.parse().map_err(|_| "--size width".to_string())?,
                    h.parse().map_err(|_| "--size height".to_string())?,
                ));
            }
            other => return Err(format!("unknown arg: {other}")),
        }
    }
    Ok(Args {
        spec: spec.ok_or("--spec is required")?,
        out: out.ok_or("--out is required")?,
        scale: if scale > 0.0 { scale } else { 1.0 },
        mode,
        script,
        size,
        vello_only,
        flow,
        focus,
    })
}
