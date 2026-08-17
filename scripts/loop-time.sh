#!/usr/bin/env bash
# scripts/loop-time.sh — the edit-to-rendered-PNG loop, as one number.
#
# Simulates the loop whoever is fixing the canvas actually runs: change some
# shell/render code, rebuild `brightfield-shot`, look at the PNG it produces.
# Rather than editing a tracked file (which would leave a diff, or race a
# sibling lane editing the same crate), it forces the same recompile with
# `cargo clean -p brightfield-shell` — that crate is a leaf (nothing else in
# the workspace depends on it as a library except brightfield-bench, which
# this script never builds), so cleaning it and rebuilding only
# `--bin brightfield-shot` reproduces exactly what an edit inside
# crates/brightfield-shell/src/** would force: a recompile of that crate and
# a relink of the one binary, with every dependency crate's own compiled
# output left untouched.
#
# Usage:
#   scripts/loop-time.sh              # debug build — the local loop's profile
#   scripts/loop-time.sh --release    # release profile
#
# First run in a fresh target/ pays the full dependency build (there is
# nothing yet for `cargo clean -p` to remove) and so reports close to a cold
# `cargo build --workspace --all-targets`. Run it a second time to see the
# steady-state incremental number — the one that recurs on every edit.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# bash 3.2 (macOS's system /bin/bash) mishandles `"${arr[@]}"` on an empty
# array under `set -u`, so the optional --release flag is plain strings
# rather than an array element, and is empty (not unset) in the debug case.
profile_flag=""
profile_dir="debug"
if [[ "${1:-}" == "--release" ]]; then
  profile_flag="--release"
  profile_dir="release"
elif [[ -n "${1:-}" ]]; then
  echo "usage: $0 [--release]" >&2
  exit 2
fi

# Same toolchain resolution as scripts/bench-baseline.sh: use the pinned
# 1.95.0 via rustup when it's installed (matching CI and the local loop),
# otherwise fall through to whatever `cargo` resolves to (a rustup directory
# override, or a rust-toolchain.toml pin).
cargo_bin="cargo"
if command -v rustup >/dev/null 2>&1 && rustup toolchain list 2>/dev/null | grep -q '^1\.95\.0'; then
  cargo_bin="rustup run 1.95.0 cargo"
fi

spec="examples/bars.yaml"
out="$(mktemp -t brightfield-loop-XXXXXX).png"
trap 'rm -f "$out"' EXIT

echo "reset:  cargo clean -p brightfield-shell"
$cargo_bin clean -p brightfield-shell >/dev/null 2>&1 || true

echo "build:  $cargo_bin build $profile_flag -p brightfield-shell --bin brightfield-shot"
echo "render: target/$profile_dir/brightfield-shot --spec $spec --out $out --vello-only"

start=$(date +%s.%N)
$cargo_bin build $profile_flag -p brightfield-shell --bin brightfield-shot
"target/$profile_dir/brightfield-shot" --spec "$spec" --out "$out" --vello-only
end=$(date +%s.%N)

if [[ ! -s "$out" ]]; then
  echo "loop-time: FAILED — no PNG produced at $out" >&2
  exit 1
fi

elapsed=$(awk -v s="$start" -v e="$end" 'BEGIN{printf "%.2f", e-s}')
size=$(du -h "$out" | cut -f1)
echo "loop-time: ${elapsed}s  (produced a ${size} PNG, then discarded)"
