//! `bf-imgdiff` — a perceptual PNG diff (pixelmatch / dify-style YIQ metric).
//!
//! Replaces the byte-exact `cmp -s` example-PNG gate: egui rasterises text with
//! antialiasing that is not bit-stable across platforms, drivers or even glyph
//! atlas packing, so byte identity reports a diff on every run and is useless as
//! a regression gate. Two images are "the same" when
//! the fraction of perceptually-different pixels is within tolerance; a per-pixel
//! YIQ colour distance decides each pixel, exactly as pixelmatch/dify do.
//!
//! Usage:
//!   bf-imgdiff A.png B.png [--threshold 0.1] [--fail-ratio 0.005] [--out diff.png]
//!
//! Exit: 0 = within tolerance, 1 = differs, 2 = usage/IO error.

use std::path::PathBuf;
use std::process::ExitCode;

/// Max squared YIQ distance between black and white (pixelmatch constant).
const MAX_DELTA: f64 = 35215.0;

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            eprintln!(
                "usage: bf-imgdiff A.png B.png [--threshold T] [--fail-ratio R] [--out D.png]"
            );
            return ExitCode::from(2);
        }
    };

    let a = match image::open(&args.left) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            eprintln!("open {}: {e}", args.left.display());
            return ExitCode::from(2);
        }
    };
    let b = match image::open(&args.right) {
        Ok(i) => i.to_rgba8(),
        Err(e) => {
            eprintln!("open {}: {e}", args.right.display());
            return ExitCode::from(2);
        }
    };

    if a.dimensions() != b.dimensions() {
        eprintln!(
            "DIMENSIONS differ: {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
        return ExitCode::from(1);
    }

    let (w, h) = a.dimensions();
    let cutoff = MAX_DELTA * args.threshold * args.threshold;
    let mut diff_pixels = 0u64;
    let mut out = args
        .out
        .as_ref()
        .map(|_| image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255])));

    for y in 0..h {
        for x in 0..w {
            let pa = a.get_pixel(x, y).0;
            let pb = b.get_pixel(x, y).0;
            let delta = color_delta(pa, pb);
            if delta > cutoff {
                diff_pixels += 1;
                if let Some(img) = out.as_mut() {
                    img.put_pixel(x, y, image::Rgba([230, 40, 40, 255]));
                }
            }
        }
    }

    let total = u64::from(w) * u64::from(h);
    let ratio = diff_pixels as f64 / total as f64;
    if let (Some(img), Some(path)) = (out, args.out.as_ref()) {
        let _ = img.save(path);
    }
    eprintln!(
        "{} diff pixels / {total} ({:.4}%), fail-ratio {:.4}%",
        diff_pixels,
        ratio * 100.0,
        args.fail_ratio * 100.0
    );
    if ratio > args.fail_ratio {
        eprintln!("DIFFERS");
        ExitCode::from(1)
    } else {
        eprintln!("OK");
        ExitCode::SUCCESS
    }
}

/// Squared YIQ colour distance between two RGBA pixels (pixelmatch). Semi-
/// transparent pixels are blended over white first.
fn color_delta(a: [u8; 4], b: [u8; 4]) -> f64 {
    let (r1, g1, b1) = blend_over_white(a);
    let (r2, g2, b2) = blend_over_white(b);
    let dy = rgb2y(r1, g1, b1) - rgb2y(r2, g2, b2);
    let di = rgb2i(r1, g1, b1) - rgb2i(r2, g2, b2);
    let dq = rgb2q(r1, g1, b1) - rgb2q(r2, g2, b2);
    0.5053 * dy * dy + 0.299 * di * di + 0.1957 * dq * dq
}

fn blend_over_white(p: [u8; 4]) -> (f64, f64, f64) {
    let a = f64::from(p[3]) / 255.0;
    let blend = |c: u8| 255.0 + (f64::from(c) - 255.0) * a;
    (blend(p[0]), blend(p[1]), blend(p[2]))
}

fn rgb2y(r: f64, g: f64, b: f64) -> f64 {
    r * 0.298_895_31 + g * 0.586_622_47 + b * 0.114_482_23
}
fn rgb2i(r: f64, g: f64, b: f64) -> f64 {
    r * 0.595_977_99 - g * 0.274_176_10 - b * 0.321_801_89
}
fn rgb2q(r: f64, g: f64, b: f64) -> f64 {
    r * 0.211_470_17 - g * 0.522_617_11 + b * 0.311_146_94
}

struct Args {
    left: PathBuf,
    right: PathBuf,
    threshold: f64,
    fail_ratio: f64,
    out: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut positional = Vec::new();
    let mut threshold = 0.1_f64;
    let mut fail_ratio = 0.005_f64;
    let mut out = None;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--threshold" => {
                threshold = it
                    .next()
                    .ok_or("--threshold needs a value")?
                    .parse()
                    .map_err(|_| "--threshold not a number".to_string())?;
            }
            "--fail-ratio" => {
                fail_ratio = it
                    .next()
                    .ok_or("--fail-ratio needs a value")?
                    .parse()
                    .map_err(|_| "--fail-ratio not a number".to_string())?;
            }
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a value")?)),
            other if other.starts_with("--") => return Err(format!("unknown flag {other}")),
            other => positional.push(PathBuf::from(other)),
        }
    }
    if positional.len() != 2 {
        return Err("need exactly two image paths".to_string());
    }
    Ok(Args {
        left: positional[0].clone(),
        right: positional[1].clone(),
        threshold,
        fail_ratio,
        out,
    })
}
