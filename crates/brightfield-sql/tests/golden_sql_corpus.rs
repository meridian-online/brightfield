//! Golden-SQL corpus harness — the byte-stability proof for the emission
//! layer.
//!
//! Walks every example spec in the repository (`examples/*.yaml`,
//! `examples/live/*.yaml`) plus the vendored observed corpus
//! (`crates/brightfield-spec/vendor/mosaic-specs/yaml/*.yaml`), emits the
//! source DDL, every mark's chart query, and every mark's rows query, and
//! folds the results into one deterministic text dump.
//!
//! The dump deliberately contains **emitted SQL and bindings only — no plan
//! hashes**: `plan_hash` is structural over the IR and may legitimately move
//! when the IR's internal shape changes, while the emitted SQL must not.
//! Nothing persists plan hashes across sessions; the SQL bytes are the
//! contract this harness pins.
//!
//! # Proving byte-stability across a refactor
//!
//! The harness is committed; golden dumps are not. To prove a branch leaves
//! emission byte-identical to a baseline:
//!
//! ```text
//! # on the baseline checkout (copy this test file in if it predates it):
//! BF_GOLDEN_SQL_WRITE=/tmp/base.sql cargo test -p brightfield-sql --test golden_sql_corpus
//! # on the branch:
//! BF_GOLDEN_SQL_EXPECT=/tmp/base.sql cargo test -p brightfield-sql --test golden_sql_corpus
//! ```
//!
//! The `EXPECT` run fails with a first-difference diff if any emitted byte
//! moved. Without either env var the tests still guard determinism and
//! corpus coverage.

use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use brightfield_spec::{parse_spec, Format};
use brightfield_sql::emit::{emit_all_queries, emit_rows_query, emit_sources};

/// Corpus roots, relative to this crate's manifest dir, with the stable label
/// each contributes to section headers (so the dump never contains absolute
/// paths).
const CORPUS_ROOTS: &[(&str, &str)] = &[
    ("../../examples", "examples"),
    ("../../examples/live", "examples/live"),
    (
        "../brightfield-spec/vendor/mosaic-specs/yaml",
        "vendor/mosaic-specs",
    ),
];

/// Every `.yaml`/`.yml` file directly under `dir` (non-recursive), sorted by
/// file name for determinism.
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for ent in entries.flatten() {
        let p = ent.path();
        if !p.is_file() {
            continue;
        }
        let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.ends_with(".yaml") || name.ends_with(".yml") {
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Emit one spec file into the dump. Every outcome — DDL, per-mark chart
/// query, per-mark rows query, and every error — is recorded, so a lowering
/// regression that turns success into error (or vice versa) also shows up as
/// a byte difference.
fn dump_spec(out: &mut String, label: &str, path: &Path) {
    let _ = writeln!(out, "=== {label} ===");
    let source = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            let _ = writeln!(out, "-- read error: {e}");
            return;
        }
    };
    let parsed = match parse_spec(&source, Format::Yaml) {
        Ok(p) => p,
        Err(e) => {
            let _ = writeln!(out, "-- parse error: {e}");
            return;
        }
    };
    let spec = parsed.spec;

    match emit_sources(&spec, None) {
        Ok(output) => {
            for (i, ddl) in output.statements.iter().enumerate() {
                let _ = writeln!(out, "-- ddl[{i}] {}: {}", ddl.view_name, ddl.sql);
            }
            for (i, w) in output.warnings.iter().enumerate() {
                let _ = writeln!(out, "-- ddl warning[{i}]: {w:?}");
            }
        }
        Err(e) => {
            let _ = writeln!(out, "-- ddl error: {e}");
        }
    }

    for (i, result) in emit_all_queries(&spec, None).iter().enumerate() {
        match result {
            Ok(q) => {
                let _ = writeln!(out, "-- mark[{i}] query: {}", q.sql);
                if !q.bindings.is_empty() {
                    let _ = writeln!(out, "-- mark[{i}] bindings: {:?}", q.bindings);
                }
                match emit_rows_query(&spec, i, None, None) {
                    Ok(r) => {
                        let _ = writeln!(out, "-- mark[{i}] rows: {}", r.sql);
                    }
                    Err(e) => {
                        let _ = writeln!(out, "-- mark[{i}] rows error: {e}");
                    }
                }
            }
            Err(e) => {
                let _ = writeln!(out, "-- mark[{i}] query error: {e}");
            }
        }
    }
}

/// Build the full corpus dump. Deterministic: corpus roots in declaration
/// order, files sorted within each root, no absolute paths, no hashes.
fn corpus_dump() -> String {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut out = String::new();
    out.push_str("-- golden SQL corpus dump: emitted SQL + bindings only (no plan hashes)\n");
    for (rel, label) in CORPUS_ROOTS {
        let dir = manifest_dir.join(rel);
        for path in yaml_files(&dir) {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            dump_spec(&mut out, &format!("{label}/{name}"), &path);
        }
    }
    out
}

/// The dump is stable across builds in-process and actually covers the
/// aggregate-emitting lowerers — the guards that keep the byte-equality
/// proof from going vacuous if the corpus moves.
#[test]
fn corpus_dump_is_deterministic_and_covers_aggregates() {
    let a = corpus_dump();
    let b = corpus_dump();
    assert_eq!(a, b, "corpus dump must be deterministic");

    let sections = a.matches("=== ").count();
    assert!(
        sections >= 30,
        "corpus unexpectedly small ({sections} specs) — did the example dirs move?"
    );
    for needle in [
        "GROUP BY",                 // grouped aggregation (density / hexbin / cell)
        "regr_slope",               // the regression sufficient-statistics family
        "__bf_count",               // the reserved occupancy aggregate
        "CAST(COUNT(*) AS DOUBLE)", // the historical count spelling, byte-exact
    ] {
        assert!(
            a.contains(needle),
            "corpus dump lost coverage of {needle:?} — the byte-stability proof \
             would no longer exercise the aggregate emission paths"
        );
    }
}

/// Env-var driven golden check: `BF_GOLDEN_SQL_WRITE=<path>` writes the dump;
/// `BF_GOLDEN_SQL_EXPECT=<path>` asserts byte equality against a previously
/// written dump. With neither set this builds the dump and passes.
#[test]
fn golden_sql_corpus_byte_equality() {
    let dump = corpus_dump();

    if let Ok(path) = std::env::var("BF_GOLDEN_SQL_WRITE") {
        fs::write(&path, &dump).unwrap_or_else(|e| panic!("writing {path}: {e}"));
    }

    if let Ok(path) = std::env::var("BF_GOLDEN_SQL_EXPECT") {
        let expected = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
        if expected != dump {
            let mismatch = expected.lines().zip(dump.lines()).position(|(e, d)| e != d);
            match mismatch {
                Some(i) => {
                    let exp_line = expected.lines().nth(i).unwrap_or("");
                    let got_line = dump.lines().nth(i).unwrap_or("");
                    panic!(
                        "golden SQL mismatch at line {}:\n  expected: {exp_line}\n  got:      {got_line}",
                        i + 1
                    );
                }
                None => panic!(
                    "golden SQL mismatch: line counts differ (expected {} lines, got {})",
                    expected.lines().count(),
                    dump.lines().count()
                ),
            }
        }
    }
}
