//! `generate-deviations` binary.
//!
//! Reads a YAML deviation registry and writes `DEVIATIONS.md`. Output is
//! stable-sorted by id, LF-terminated on every platform (we write bytes
//! directly rather than using `writeln!` to avoid CRLF translation on
//! Windows), and deterministic — byte-identical on repeated runs.
//!
//! Args:
//!   --input  PATH   default ./deviations.yaml
//!   --output PATH   default ./DEVIATIONS.md

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_conformance::{load_deviations, Deviation, DeviationRegistry};

fn main() -> ExitCode {
    let (input, output) = match parse_args() {
        Ok(pair) => pair,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!("usage: generate-deviations [--input PATH] [--output PATH]");
            return ExitCode::from(2);
        }
    };
    let registry = match load_deviations(&input) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("failed to load {input:?}: {e}");
            return ExitCode::from(1);
        }
    };
    let bytes = render(&registry);
    if let Err(e) = write_lf_bytes(&output, &bytes) {
        eprintln!("failed to write {output:?}: {e}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

fn parse_args() -> Result<(PathBuf, PathBuf), String> {
    let mut input = PathBuf::from("deviations.yaml");
    let mut output = PathBuf::from("DEVIATIONS.md");
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--input" => {
                input = it
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--input needs a value".to_string())?;
            }
            "--output" => {
                output = it
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--output needs a value".to_string())?;
            }
            "-h" | "--help" => return Err("help".to_string()),
            other => return Err(format!("unrecognised arg: {other}")),
        }
    }
    Ok((input, output))
}

/// Render the registry to Markdown bytes. Deterministic: same registry
/// in → byte-identical output.
fn render(registry: &DeviationRegistry) -> Vec<u8> {
    let mut out = String::new();
    out.push_str("# Deviations\n\n");
    out.push_str("Deliberate divergences from Mosaic-web rendering. Generated from\n");
    out.push_str("`deviations.yaml` by `cargo run --bin generate-deviations`.\n");
    out.push('\n');
    if registry.is_empty() {
        out.push_str("(no deviations registered)\n");
        return out.into_bytes();
    }
    let mut sorted: Vec<&Deviation> = registry.iter().collect();
    sorted.sort_by(|a, b| a.id.cmp(&b.id));
    for dev in sorted {
        render_one(&mut out, dev);
    }
    out.into_bytes()
}

fn render_one(out: &mut String, d: &Deviation) {
    out.push_str("## ");
    out.push_str(&d.id);
    out.push_str(" — ");
    out.push_str(&d.surface);
    out.push_str("\n\n");
    out.push_str("**Mosaic behaviour.** ");
    out.push_str(&d.mosaic_behaviour);
    out.push_str("\n\n");
    out.push_str("**Brightfield behaviour.** ");
    out.push_str(&d.brightfield_behaviour);
    out.push_str("\n\n");
    out.push_str("**Rationale.** ");
    out.push_str(&d.rationale);
    out.push_str("\n\n");
    if !d.affected_specs.is_empty() {
        out.push_str("**Affected specs:** ");
        let mut specs = d.affected_specs.clone();
        specs.sort();
        out.push_str(&specs.join(", "));
        out.push_str("\n\n");
    }
    if !d.conformance_layers_suppressed.is_empty() {
        out.push_str("**Conformance layers suppressed:** ");
        let mut layers: Vec<String> = d
            .conformance_layers_suppressed
            .iter()
            .map(|l| (*l as u8).to_string())
            .collect();
        layers.sort();
        out.push_str(&layers.join(", "));
        out.push_str("\n\n");
    }
}

/// Write `bytes` to `path` using LF line endings — we already constructed
/// the buffer with `\n`, so this is just a binary write (no text-mode
/// translation). Ensures determinism across platforms.
fn write_lf_bytes(path: &PathBuf, bytes: &[u8]) -> std::io::Result<()> {
    let mut f = fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}
