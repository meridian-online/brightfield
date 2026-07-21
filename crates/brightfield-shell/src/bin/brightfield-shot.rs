//! `brightfield-shot` — headless real-UI capture (Tier-2 of the loop).
//!
//! Boots the real egui shell over a spec, renders the whole window through
//! egui_wgpu into an offscreen texture, and writes a PNG — no display required.
//! This is the capability the gpui host never had: an agent edits the UI and
//! sees the actual pixels, including scripted keystrokes driving the view.
//!
//! The window holds both views; the spec decides which of them opens:
//! - a **Mosaic dashboard** (`--spec dashboard.yaml`) → the chart workbench;
//! - a **Protocol manifest** (`arcform.yaml`, with `BRIGHTFIELD_PROTOCOL_OFFLINE=1`)
//!   → the Protocol panel (dock + DAG + outline + inspector + steps).
//!
//! One capture path serves both. This binary used to sniff the spec itself and
//! branch into a second capture function over a second shell — the same fork
//! the live window carried, spelled again.
//!
//! Usage:
//!
//! ```text
//! brightfield-shot --spec S.yaml --out out.png
//!                  [--size WxH] [--scale N] [--theme light|dark]
//!                  [--script keys.ndjson]
//!                  [--flow vertical|horizontal] [--focus <dotted-id>]
//! ```
//!
//! `--flow` / `--focus` apply to the Protocol panel: the reading axis (vertical
//! by default) and the boot selection (the dotted asset id a click would target,
//! so a scripted `za`/`Enter` has a cursor to act on).

use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::{capture_png, capture_vello_only, parse_script};
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_workbench::ViewKind;

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

    let boot = match Boot::open(&args.spec, args.flow, args.focus.clone()) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    eprintln!("{} from {}", boot.describe(), args.spec);
    if boot.view == ViewKind::Protocol {
        // Surface the family tile ids so `--focus` has a target for a scripted `za`.
        for (id, node) in &boot.protocol.graph_collapsed.nodes {
            if node.kind == brightfield_protocol::graph::AssetKind::Family {
                eprintln!("  family tile: {id}");
            }
        }
    }

    let _ = args.size; // reserved override; the window auto-sizes today.
    if args.vello_only {
        // The Vello-only raster is the *dashboard* composite with no egui around
        // it, and there is no protocol analogue of it. Collapsing the fork made
        // this combination reachable for the first time, so it gets an answer
        // rather than an empty PNG.
        if boot.view != ViewKind::Charts {
            eprintln!(
                "error: --vello-only renders a composed dashboard; {} opens the Protocol view",
                args.spec
            );
            return ExitCode::from(2);
        }
        return report(
            capture_vello_only(boot.composed, args.scale, &args.out),
            &args.out,
        );
    }
    report(
        capture_png(boot, args.mode, args.scale, &args.out, script),
        &args.out,
    )
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
            "--scale" => {
                scale = next()?
                    .parse()
                    .map_err(|_| "--scale not a number".to_string())?
            }
            "--theme" => {
                mode = if next()? == "dark" {
                    Mode::Dark
                } else {
                    Mode::Light
                }
            }
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
