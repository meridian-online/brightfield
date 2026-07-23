#!/usr/bin/env bash
# Perceptual example-PNG gate. Byte-exact `cmp -s` no longer works: egui text
# antialiasing is not bit-stable across platforms/drivers, so byte identity
# flags a diff on every run.
#
# Renders every example spec headlessly (`brightfield-shot --vello-only`, the
# composed-dashboard canvas render) and compares each against a baseline
# directory with a pixelmatch/dify-style perceptual diff (`bf-imgdiff`) instead
# of byte identity. Capture a baseline from a known-good commit first:
#
#   mkdir -p /tmp/bf-baselines
#   for f in examples/*.yaml; do \
#     cargo run -q -p brightfield-shell --bin brightfield-shot -- \
#       --spec "$f" --out /tmp/bf-baselines/$(basename "$f" .yaml).png --vello-only; done
#   git switch <feature-branch>
#   scripts/compare-example-pngs.sh /tmp/bf-baselines
#
# Examples with no baseline PNG (new in the branch under test) are reported as
# NEW and do not fail the run. Any DIFFERS (perceptual) is a halt condition.
#
# Tunables: FAIL_RATIO (fraction of differing pixels tolerated, default 0.005),
# THRESHOLD (per-pixel YIQ cutoff, default 0.1).

set -euo pipefail

baseline_dir="${1:?usage: $0 <baseline-dir>}"
fail_ratio="${FAIL_RATIO:-0.005}"
threshold="${THRESHOLD:-0.1}"
out_dir="$(mktemp -d)"
trap 'rm -rf "$out_dir"' EXIT

# Build the renderer + the perceptual differ once so the loop is fast.
cargo build -q -p brightfield-shell --bin brightfield-shot
cargo build -q -p brightfield-shell --bin bf-imgdiff
imgdiff="$(cargo build -q -p brightfield-shell --bin bf-imgdiff --message-format=json 2>/dev/null \
    | sed -n 's/.*"executable":"\([^"]*bf-imgdiff\)".*/\1/p' | head -1)"
imgdiff="${imgdiff:-target/debug/bf-imgdiff}"

identical=0 differing=0 new=0
for f in examples/*.yaml; do
    name="$(basename "$f" .yaml)"
    cargo run -q -p brightfield-shell --bin brightfield-shot -- \
        --spec "$f" --out "$out_dir/$name.png" --vello-only \
        >/dev/null 2>"$out_dir/$name.stderr" \
        || { echo "RENDER FAILED: $name — stderr:" >&2; cat "$out_dir/$name.stderr" >&2; exit 1; }
    if [ ! -f "$baseline_dir/$name.png" ]; then
        echo "NEW:      $name (no baseline)"
        new=$((new + 1))
    elif "$imgdiff" "$baseline_dir/$name.png" "$out_dir/$name.png" \
            --threshold "$threshold" --fail-ratio "$fail_ratio" >/dev/null 2>&1; then
        identical=$((identical + 1))
    else
        echo "DIFFERS:  $name (perceptual)"
        differing=$((differing + 1))
    fi
done

echo "within-tolerance: $identical, differing: $differing, new: $new"
[ "$differing" -eq 0 ]
