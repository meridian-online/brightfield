//! `conformance` binary.
//!
//! CLI entrypoint for the conformance runner. Dispatches through the same
//! `LayerCheck` registry `cargo test` uses, so adding a layer doesn't fork
//! test vs CLI code paths.
//!
//! Args:
//!   --layers CSV   layer numbers in 1..=4; default 1,2,3,4
//!   --corpus KIND  `curated` | `observed`; default `curated`
//!
//! Exit code: non-zero iff any `LayerOutcome::Fail` is present in the
//! report.

use std::convert::TryFrom;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

use brightfield_conformance::{
    load_deviations, run_conformance, ConformanceLayer, Corpus, DeviationRegistry, LayerOutcome,
};

fn main() -> ExitCode {
    let (layers, corpus) = match parse_args() {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("error: {msg}");
            eprintln!(
                "usage: conformance [--layers 1,2,3,4] [--corpus curated|observed]"
            );
            return ExitCode::from(2);
        }
    };
    let registry_path = PathBuf::from("deviations.yaml");
    let registry = if registry_path.exists() {
        match load_deviations(&registry_path) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("failed to load {registry_path:?}: {e}");
                return ExitCode::from(1);
            }
        }
    } else {
        DeviationRegistry::default()
    };
    let report = run_conformance(corpus, &layers, &registry);
    for rec in &report.records {
        println!(
            "{}\tlayer {}\t{}",
            rec.spec_name,
            rec.layer.as_u8(),
            outcome_tag(&rec.outcome)
        );
    }
    let s = report.summary;
    println!(
        "SUMMARY: passed={} failed={} suppressed={} pending={}",
        s.passed, s.failed, s.suppressed, s.pending
    );
    if s.failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}

fn outcome_tag(o: &LayerOutcome) -> String {
    match o {
        LayerOutcome::Pass => "pass".to_string(),
        LayerOutcome::Fail { details } => format!("FAIL: {details}"),
        LayerOutcome::Suppressed { deviation_id } => {
            format!("suppressed({deviation_id})")
        }
        LayerOutcome::Pending { reason } => format!("pending: {reason}"),
    }
}

fn parse_args() -> Result<(Vec<ConformanceLayer>, Corpus), String> {
    let mut layers_csv: Option<String> = None;
    let mut corpus: Corpus = Corpus::Curated;
    let mut it = env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--layers" => {
                layers_csv = Some(
                    it.next()
                        .ok_or_else(|| "--layers needs a value".to_string())?,
                );
            }
            "--corpus" => {
                let v = it
                    .next()
                    .ok_or_else(|| "--corpus needs a value".to_string())?;
                corpus = match v.as_str() {
                    "curated" => Corpus::Curated,
                    "observed" => Corpus::Observed,
                    other => return Err(format!("--corpus: unknown kind {other}")),
                };
            }
            "-h" | "--help" => return Err("help".to_string()),
            other => return Err(format!("unrecognised arg: {other}")),
        }
    }
    let csv = layers_csv.unwrap_or_else(|| "1,2,3,4".to_string());
    let mut layers: Vec<ConformanceLayer> = Vec::new();
    for tok in csv.split(',') {
        let t = tok.trim();
        if t.is_empty() {
            continue;
        }
        let n: u8 = t
            .parse()
            .map_err(|_| format!("--layers: not a number: {t}"))?;
        let layer = ConformanceLayer::try_from(n)
            .map_err(|_| format!("--layers: out of range 1..=4: {n}"))?;
        layers.push(layer);
    }
    if layers.is_empty() {
        return Err("--layers: empty list".to_string());
    }
    Ok((layers, corpus))
}
