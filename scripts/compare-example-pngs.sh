#!/usr/bin/env bash
# fww_ac08 (card 0016): byte-identity harness for the example PNGs.
#
# Renders every example spec headlessly (BRIGHTFIELD_DUMP_PNG) and byte-
# compares each against a baseline directory captured from a known-good
# commit (typically main):
#
#   mkdir -p /tmp/bf-baselines
#   for f in examples/*.yaml; do \
#     BRIGHTFIELD_DUMP_PNG=/tmp/bf-baselines/$(basename "$f" .yaml).png \
#       cargo run -q -p brightfield-app -- "$f"; done
#   git switch <feature-branch>
#   scripts/compare-example-pngs.sh /tmp/bf-baselines
#
# Examples with no baseline PNG (new in the branch under test) are reported
# as NEW and do not fail the run. Any DIFFERS is a halt condition.
#
# Known caveat: the raster family (raster, raster-blues, raster-legend) is
# not byte-stable run-to-run even on an unmodified checkout — DuckDB's
# GROUP BY row order varies, so anti-aliased cell edges blend in a different
# draw order (~0.1% of pixels, small deltas). A DIFFERS on those examples
# must be re-checked against two runs of the SAME commit before it is
# treated as a regression.

set -euo pipefail

baseline_dir="${1:?usage: $0 <baseline-dir>}"
out_dir="$(mktemp -d)"
trap 'rm -rf "$out_dir"' EXIT

identical=0 differing=0 new=0
for f in examples/*.yaml; do
    name="$(basename "$f" .yaml)"
    BRIGHTFIELD_DUMP_PNG="$out_dir/$name.png" cargo run -q -p brightfield-app -- "$f" >/dev/null 2>&1
    if [ ! -f "$baseline_dir/$name.png" ]; then
        echo "NEW:      $name (no baseline)"
        new=$((new + 1))
    elif cmp -s "$baseline_dir/$name.png" "$out_dir/$name.png"; then
        identical=$((identical + 1))
    else
        echo "DIFFERS:  $name"
        differing=$((differing + 1))
    fi
done

echo "identical: $identical, differing: $differing, new: $new"
[ "$differing" -eq 0 ]
