//! `brightfield-shot` — headless real-UI capture (Tier-2 of the loop).
//!
//! Boots the real egui shell over a Mosaic spec, renders the whole window
//! (Vello canvas + native chrome) through egui_wgpu into an offscreen texture,
//! and writes a PNG — no display required. This is the capability the gpui host
//! never had: an agent edits the UI and sees the actual pixels.
//!
//! Usage:
//!   brightfield-shot --spec examples/dashboard.yaml --out out.png
//!                    [--size WxH] [--scale N] [--theme light|dark]
//!                    [--script keys.ndjson]
//!
//! `--size` overrides the auto window size (rarely needed — the shell sizes
//! itself to the dashboard + chrome).

use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_shell::capture::{capture_png, capture_vello_only, parse_script};
use brightfield_shell::design::Mode;
use brightfield_shell::pipeline::compose_spec;

struct Args {
    spec: String,
    out: PathBuf,
    scale: f32,
    mode: Mode,
    script: Option<PathBuf>,
    size: Option<(u32, u32)>,
    vello_only: bool,
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: brightfield-shot --spec S.yaml --out O.png \
                 [--scale N] [--theme light|dark] [--script keys.ndjson] [--size WxH]"
            );
            return ExitCode::from(2);
        }
    };

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

    let _ = args.size; // reserved override; the shell auto-sizes today.
    let result = if args.vello_only {
        capture_vello_only(composed, args.scale, &args.out)
    } else {
        capture_png(composed, args.mode, args.scale, &args.out, script)
    };
    match result {
        Ok((w, h)) => {
            eprintln!("wrote {} ({w}x{h} device px)", args.out.display());
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
    })
}
