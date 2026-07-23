#!/usr/bin/env bash
# The measured performance baseline, as one command:
#
#   ./scripts/bench-baseline.sh              # the full baseline (release build)
#   ./scripts/bench-baseline.sh --quick      # a fast smoke pass
#   ./scripts/bench-baseline.sh --skip-frames    # engine suites only (no GPU)
#
# Everything after the script name is passed through to the harness — see
# `cargo run --release -p brightfield-bench -- --help-ish` usage in
# crates/brightfield-bench/src/main.rs.
#
# Results land in benchmarks/results/ as a JSON record plus a generated
# Markdown summary, named <date>-<machine>. Re-measuring after an engine
# change is running this script again on the same machine and comparing the
# two records; the scenario specs are compiled into the harness from
# benchmarks/specs/, so the committed scenario and the executed scenario
# cannot drift apart.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Release is the measurement profile: a debug number is not a baseline. The
# harness itself warns if it was somehow built without optimisations.
#
# The workspace floor is rust-version 1.95 and CI pins 1.95.0 exactly; a
# machine whose default toolchain is older refuses the build outright. When
# rustup has the pinned toolchain, use it — same resolution CI makes.
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^1\.95\.0'; then
  exec rustup run 1.95.0 cargo run --release -p brightfield-bench -- "$@"
fi
exec cargo run --release -p brightfield-bench -- "$@"
