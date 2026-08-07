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
//!                  [--script keys.ndjson] [--force-sample N]
//!                  [--flow vertical|horizontal] [--focus <dotted-id>]
//! brightfield-shot --gallery <component-id> --out out.png
//!                  [--size WxH] [--scale N] [--theme light|dark]
//! ```
//!
//! `--flow` / `--focus` apply to the Protocol panel: the reading axis (vertical
//! by default) and the boot selection (the dotted asset id a click would target,
//! so a scripted `za`/`Enter` has a cursor to act on).
//!
//! `--force-sample N` draws one row in N — a power of two — through the same
//! pushed-down clause the renderer reaches for above its drawable ceiling, and
//! draws the sampling notice in the plot's own ink. It exists so ONE spec can
//! produce a complete PNG and a sampled PNG over the SAME rows: judging the
//! treatment against a different, denser dataset would confound it with the
//! density. Combine with `--vello-only` to see what a chart-only export keeps
//! — the notice is in the scene precisely so it travels with the chart, which
//! a banner would not.
//!
//! A colour channel is sampled like any other: the domain order is the
//! category's own text rather than the order the rows arrived in, and the
//! unsampled query supplies the value set, so a class the sample drops keeps
//! its palette slot for the classes after it.
//!
//! It still **refuses** a plot with a band axis or a sequential ramp, and says
//! which channel. Those domains are read off the rows that were drawn and the
//! unsampled query's value set does not put them back — a band domain's order
//! is where the marks are, and a ramp is anchored to the drawn extent — so the
//! sampled render would place or colour a value differently from the complete
//! one, under a notice that mentions only the dropped rows.
//!
//! # A degraded protocol render says so, on stderr
//!
//! A protocol whose models could not all be read draws what it can — one
//! opaque chip in place of the statements inside the step — rather than
//! refusing. That is deliberate, and it is a failure mode an unattended caller
//! could not otherwise see: the picture looks finished, the step count is
//! unchanged, and the node counts settle nothing either — a caller holding one
//! render has nothing telling it what those totals should have been. The
//! figures behind that are in
//! [`brightfield_shell::protocol::ProtocolInputs::degrade_report`], each
//! against the fixture that produced it.
//!
//! So the boot summary carries a `, N degraded` clause, and one line per
//! degraded node precedes it — `N` counts nodes, and a single fault can badge
//! more than one:
//!
//! ```text
//! S.yaml: degraded step tier: unreadable: models/t.sql: Permission denied (os error 13) — …
//! protocol acc (5 collapsed / 5 full nodes, 3 steps, Vertical flow, 1 degraded) from S.yaml
//! ```
//!
//! A degrade raised by the model read leads with its class — `absent`,
//! `unreadable` or `unavailable`. What each one means, and how much it commits
//! to about the cause, is [`brightfield_protocol::Degradation`]; the rest of
//! the line is that class's own guidance. A degrade raised anywhere else carries the
//! underlying message with no class tag; `examples/protocol/degrade.yaml` is
//! one, and its line opens `degraded step transform: SQL parse error`.
//!
//! **The exit code stays 0 and the PNG is still written.** On this binary a
//! non-zero exit means *no image was produced* — `report` below implements it,
//! mapping `Ok` to success and `Err` to 1. Spending it on a
//! degraded-but-drawn render would tell a caller its capture failed when a file
//! exists on disk, and would contradict the degrade's own design — draw what
//! you can. The stderr line is the channel; a caller that wants to fail on a
//! partial render matches it.
//!
//! `--gallery` renders one design-gallery component solo through the same
//! deterministic capture path — an agent that just changed a primitive can
//! see it without composing a dashboard. On this path `--size` is honoured
//! (defaulting to the component's own solo size); on the `--spec` path it
//! remains a reserved override, because there the window derives from the
//! document.

use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_protocol::layout::Flow;
use brightfield_shell::capture::{
    capture_component, capture_png, capture_vello_only, parse_script,
};
use brightfield_shell::design::Mode;
use brightfield_shell::window::Boot;
use brightfield_sql::ir::SampleRate;
use brightfield_workbench::ViewKind;

struct Args {
    spec: Option<String>,
    gallery: Option<String>,
    out: PathBuf,
    scale: f32,
    mode: Mode,
    script: Option<PathBuf>,
    size: Option<(u32, u32)>,
    vello_only: bool,
    flow: Flow,
    focus: Option<String>,
    /// `--force-sample N`: draw one row in N, with the sampling notice, at a
    /// row count the spec did not have to change to produce.
    force_sample: Option<SampleRate>,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: brightfield-shot --spec S.yaml --out O.png \
                 [--scale N] [--theme light|dark] [--script keys.ndjson] \
                 [--size WxH] [--force-sample N] [--flow vertical|horizontal] \
                 [--focus <dotted-id>]\n\
                 \x20      brightfield-shot --gallery <component-id> --out O.png \
                 [--scale N] [--theme light|dark] [--size WxH]"
            );
            return ExitCode::from(2);
        }
    };

    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }

    if let Some(id) = &args.gallery {
        let size = args.size.map(|(w, h)| (w as f32, h as f32));
        eprintln!("gallery component {id}");
        return report(
            capture_component(id, args.mode, args.scale, size, &args.out),
            &args.out,
        );
    }
    // `parse_args` guarantees one of the two modes is present.
    let spec = args
        .spec
        .as_deref()
        .expect("--spec or --gallery is present");

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

    let boot = match Boot::open_sampled(spec, args.flow, args.focus.clone(), args.force_sample) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(1);
        }
    };
    // `Boot::open` always names a view, so the fallback here is unreachable —
    // said at the call site rather than baked into the getter, for the reason
    // `Boot::view_or` records.
    let view = boot.view_or(ViewKind::Charts);
    eprintln!("{} from {}", boot.describe(view), spec);
    if view == ViewKind::Protocol {
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
        if view != ViewKind::Charts {
            eprintln!(
                "error: --vello-only renders a composed dashboard; {spec} opens the Protocol view"
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
    let mut gallery: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut scale = 2.0_f32;
    let mut mode = Mode::Light;
    let mut script = None;
    let mut size = None;
    let mut vello_only = false;
    let mut flow = Flow::Vertical;
    let mut focus = None;
    let mut force_sample = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        let mut next = || it.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--spec" => spec = Some(next()?),
            "--gallery" => gallery = Some(next()?),
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
            "--force-sample" => {
                let raw: u32 = next()?
                    .parse()
                    .map_err(|_| "--force-sample needs a positive integer".to_string())?;
                force_sample = Some(SampleRate::from_modulus(raw).ok_or_else(|| {
                    format!(
                        "--force-sample {raw} is not a power of two. The modulus must be one, \
                         and it is not a formality: power-of-two moduli NEST, so halving the \
                         rate can only ADD points and a later rate change densifies the picture \
                         instead of redrawing it. Rounding {raw} silently would make that true \
                         while the stated rate was a lie."
                    )
                })?);
            }
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
    if spec.is_none() && gallery.is_none() {
        return Err("one of --spec or --gallery is required".to_string());
    }
    if spec.is_some() && gallery.is_some() {
        return Err(
            "--spec and --gallery are exclusive: a shot is of a document or of a component"
                .to_string(),
        );
    }
    Ok(Args {
        spec,
        gallery,
        out: out.ok_or("--out is required")?,
        scale: if scale > 0.0 { scale } else { 1.0 },
        mode,
        script,
        size,
        vello_only,
        flow,
        focus,
        force_sample,
    })
}
