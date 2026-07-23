#!/usr/bin/env bash
# Borrowed-benchmark gate: stop upstream performance figures being quoted as ours.
#
# This project's spec grammar and architecture follow Mosaic, whose published
# performance figures (0.01s interaction latency on billion-row datasets, 60+
# FPS, sub-100ms filter response) are for a coordinator that has been built and
# benchmarked upstream. Adopting the thesis is not the same as inheriting the
# numbers: nothing in this repository may state those figures as a property of
# THIS engine. Our own numbers live in benchmarks/results/, each with its
# date, machine, dataset and methodology — measured here, or not stated.
#
# Usage (no arguments, from anywhere inside the repo):
#
#   ./scripts/check-borrowed-benchmarks.sh
#
# Exit codes:
#   0  clean
#   1  one or more violations found
#   2  the gate could not run correctly (malformed allowlist)
#
# Its own regression test is scripts/check-borrowed-benchmarks-selftest.sh.
#
# COVERED: the content of TRACKED files, via `git grep`. NOT COVERED: commit
# messages, PR text, branch names, and history — watch those yourself.
#
# False positives: a file may quote a figure legitimately (for instance, a
# results record that MEASURED sixty frames a second here). Such a file goes in
# scripts/borrowed-benchmarks-allowlist.txt as `path<TAB>reason` — an entry
# without a reason is a malformed allowlist, and the gate refuses to run.
set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 2

ALLOWLIST="scripts/borrowed-benchmarks-allowlist.txt"

# The gate's own machinery may spell the patterns it hunts; results records are
# measured data, not claims.
SELF_EXCLUDE=(
  ":(exclude)scripts/check-borrowed-benchmarks.sh"
  ":(exclude)scripts/check-borrowed-benchmarks-selftest.sh"
  ":(exclude)$ALLOWLIST"
  ":(exclude)benchmarks/results"
)

# The inherited figures, in the shapes they were quoted in. Deliberately
# narrow: "60 fps" without the plus is a vsync fact, "100ms" without "sub-" is
# a budget — neither is Mosaic's claim.
PATTERNS=(
  '60\+ ?fps'
  'sub-?100 ?ms'
  '0\.01 ?s(econds?)? (interaction |filter )?(latency|response)'
  'billion[- ](row|record|element)'
)

allowed=()
if [ -f "$ALLOWLIST" ]; then
  while IFS= read -r line; do
    case "$line" in
      ''|'#'*) continue ;;
    esac
    path=${line%%$'\t'*}
    reason=${line#*$'\t'}
    if [ "$path" = "$line" ] || [ -z "$reason" ]; then
      echo "MALFORMED ALLOWLIST ENTRY (need path<TAB>reason): $line" >&2
      exit 2
    fi
    if ! git ls-files --error-unmatch "$path" >/dev/null 2>&1; then
      echo "STALE ALLOWLIST ENTRY (not a tracked file): $path" >&2
      exit 2
    fi
    allowed+=("$path")
  done < "$ALLOWLIST"
fi

fail=0
for pat in "${PATTERNS[@]}"; do
  hits=$(git grep -iInE "$pat" -- . "${SELF_EXCLUDE[@]}" 2>/dev/null)
  [ -z "$hits" ] && continue
  while IFS= read -r hit; do
    file=${hit%%:*}
    skip=0
    for a in "${allowed[@]:-}"; do
      [ "$file" = "$a" ] && skip=1 && break
    done
    [ "$skip" = 1 ] && continue
    if [ "$fail" = 0 ]; then
      echo "Borrowed benchmark figures found. These are upstream Mosaic numbers;" >&2
      echo "this repository quotes only figures measured here (benchmarks/results/)." >&2
      echo >&2
    fi
    echo "  [$pat] $hit" >&2
    fail=1
  done <<< "$hits"
done

if [ "$fail" = 1 ]; then
  echo >&2
  echo "Fix: state the property, not the inherited number — or measure it:" >&2
  echo "  ./scripts/bench-baseline.sh" >&2
  exit 1
fi
exit 0
