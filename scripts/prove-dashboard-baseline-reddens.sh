#!/usr/bin/env bash
# Prove the generated dashboard's baseline reddens when the generator's tile
# choice moves, by moving it.
#
# crates/brightfield-shell/tests/dashboard_baseline.rs holds what the generator
# chose for a table it had never met, twice over one committed file: as the
# kind each column was given, by name, and as the picture those choices compose
# into. The picture reddens on a font bump as loudly as on a moved choice, so
# the half that has to fail legibly is the structural one, and two of its tests
# carry the claim:
#
#   each_column_of_the_table_gets_the_tile_its_type_earns
#       compares the kind chosen per column, by name, against the kinds the
#       baseline was recorded with.
#   the_preference_between_applicable_kinds_is_the_registrys_declaration_order
#       compares the order dashboard::single_column_kinds() answers in against
#       the order the baseline pins. The chooser takes the first kind whose slots
#       accept a column, so that order is the tiebreak between two applicable
#       kinds.
#
# A baseline nobody has watched fail is one a reviewer learns to re-record, and
# no other script under scripts/ re-runs the break. This one does: it copies the
# tree, moves the choice in the copy in each of the two ways below, and asks
# for a red run that names the test and prints what moved.
#
#   1. One column's recorded kind is pinned to a different kind, for the one
#      column `region`, in dashboard.rs where tile_for turns the chooser's
#      answer into the tile. The three kinds a lone column can fill declare
#      disjoint slot types, so pinning what kind_for RETURNS to a kind that does
#      not accept the column builds no tile at all and drops the column instead;
#      pinning the recorded kind moves one column and leaves the others, which
#      is what the per-column comparison claims to notice.
#   2. Two of the kinds single_column_kinds() returns swap places in the
#      registry's declaration order, in chart_kinds.rs. The kinds' slot types
#      are disjoint, so no tile and no pixel moves in the fixture: of the tests
#      in the dashboard_baseline binary, the order test is the one that goes red
#      for this break.
#
# Both directions, like the other gate self-tests here. The intact copy must
# PASS both tests, each break must turn its test red with the message that names
# what moved, and the intact copy must pass again once both breaks are put back.
# A run that goes red for another reason is not a caught break: the test has to
# be named as FAILED, so a break that stops the build from compiling does not
# count, and a test that is renamed or filtered out of the run is a failure
# rather than a pass over nothing.
#
# What it touches. The tree is copied out of `git archive HEAD` (or, with
# --tree, out of a directory) into a temporary directory that a trap removes,
# and it builds there with its own target directory. The checkout it was run
# from is only read; the run ends by reading its state again and failing if
# the state moved.
#
# Cost. Each break rebuilds brightfield-shell's test binary, and the first
# build is cold, so this is not a per-pull-request step and is not wired into
# test.yml. Point BRIGHTFIELD_PROVE_TARGET_DIR at a directory outside the
# checkout to keep the build between runs.
#
# Usage:
#   scripts/prove-dashboard-baseline-reddens.sh
#   scripts/prove-dashboard-baseline-reddens.sh --tree DIR
#
#   --tree DIR   prove the tree in DIR instead of a copy of HEAD, for a change
#                that is not committed yet. DIR is copied and never written.
#
# Exit status: 0 when every case held, 1 when one did not or when a break's
# anchor no longer matches the source it rewrites, 2 for a bad argument.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "${HERE}/.." && pwd)

TEST_BINARY=dashboard_baseline
CHOICE_TEST=each_column_of_the_table_gets_the_tile_its_type_earns
ORDER_TEST=the_preference_between_applicable_kinds_is_the_registrys_declaration_order
TILE_FILE=crates/brightfield-shell/src/dashboard.rs
REGISTRY_FILE=crates/brightfield-shell/src/chart_kinds.rs

die() {
  echo "prove-dashboard-baseline-reddens: $*" >&2
  exit 2
}

tree=""
while [ $# -gt 0 ]; do
  case "$1" in
    --tree)
      [ $# -ge 2 ] || die "--tree takes a directory"
      tree="$2"
      shift 2
      ;;
    -h | --help)
      sed -n '2,/^set -euo pipefail/p' "${BASH_SOURCE[0]}" | sed '$d' | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-baseline-proof.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

SRC="$TMP/src"
TARGET="${BRIGHTFIELD_PROVE_TARGET_DIR:-$TMP/target}"
out="$TMP/out"
fails=0

# What the checkout looks like from outside: the commit, and every file git
# would list as changed, untracked or ignored. Read twice, before the copy and
# after the last run, so "left as it was found" is something the run observed.
checkout_state() {
  git -C "$ROOT" rev-parse HEAD
  git --no-optional-locks -C "$ROOT" status --porcelain=v1 \
    --untracked-files=all --ignored=matching
}

# The pass/fail helpers print in the shape of the sibling self-tests.
pass() { echo "  ok   $1"; }
flunk() {
  echo "  FAIL $1"
  fails=$((fails + 1))
}
show_tail() { tail -n 40 "$out" | sed 's/^/       /'; }

# replace_once FILE OLD NEW — the anchor must occur exactly once. A break whose
# anchor no longer matches has stopped breaking anything, and it has to say so
# before a build is spent on it rather than report a green run as a miss.
replace_once() {
  python3 - "$1" "$2" "$3" <<'PY'
import sys

path, old, new = sys.argv[1:4]
with open(path, encoding="utf-8") as handle:
    text = handle.read()
count = text.count(old)
if count != 1:
    sys.exit(f"the anchor occurs {count} times in {path}, not once:\n{old}")
with open(path, "w", encoding="utf-8") as handle:
    handle.write(text.replace(old, new))
PY
}

# The two structural tests, by exact name, in the copy. `--exact` keeps a
# filter that happens to be a substring of another test's name from widening
# the run, and UPDATE_SNAPSHOTS is unset so nothing here can re-record.
run_tests() {
  (
    cd "$SRC"
    env -u UPDATE_SNAPSHOTS CARGO_TARGET_DIR="$TARGET" \
      cargo test --locked -p brightfield-shell --test "$TEST_BINARY" \
      -- --exact "$CHOICE_TEST" "$ORDER_TEST"
  ) > "$out" 2>&1
}

# libtest prints one `test NAME ... STATUS` line per test it ran.
reported() { grep -qxF "test $1 ... $2" "$out"; }

expect_intact() {
  local name="$1"
  if ! run_tests; then
    flunk "$name: expected a green run, got a red one"
    show_tail
    return
  fi
  local t
  for t in "$CHOICE_TEST" "$ORDER_TEST"; do
    if ! reported "$t" ok; then
      flunk "$name: the run was green without ${t} reporting ok"
      show_tail
      return
    fi
  done
  pass "$name"
}

# expect_caught NAME TEST PATTERN... — the run is red, TEST is the one named
# FAILED, and every PATTERN (an extended regex) matches a line the failure
# printed. The patterns are what say WHICH thing moved: a red run over the
# wrong assertion would satisfy the first two and none of these.
expect_caught() {
  local name="$1" test="$2" pattern
  shift 2
  if run_tests; then
    flunk "$name: expected a red run, got a green one"
    show_tail
    return
  fi
  if ! reported "$test" FAILED; then
    flunk "$name: the run went red without ${test} failing"
    show_tail
    return
  fi
  for pattern in "$@"; do
    if ! grep -qE -- "$pattern" "$out"; then
      flunk "$name: ${test} failed without printing ${pattern}"
      show_tail
      return
    fi
  done
  pass "$name — caught by ${test}"
  grep -E '^ *(left|right): ' "$out" | cut -c1-220 | sed 's/^/         /'
}

# --- the copy ---------------------------------------------------------------

before=$(checkout_state)

mkdir -p "$SRC"
if [ -n "$tree" ]; then
  [ -d "$tree" ] || die "no such directory: $tree"
  tar -C "$tree" --exclude=./target --exclude=./.git -cf - . | tar -C "$SRC" -xf -
  echo "proving the tree in ${tree}"
else
  if [ -n "$(git --no-optional-locks -C "$ROOT" status --porcelain --untracked-files=no)" ]; then
    echo "note: the checkout has uncommitted changes, which HEAD does not carry;"
    echo "      pass --tree . to prove them"
  fi
  git -C "$ROOT" archive HEAD | tar -C "$SRC" -xf -
  echo "proving HEAD ($(git -C "$ROOT" rev-parse --short HEAD))"
fi

for f in "$TILE_FILE" "$REGISTRY_FILE" "crates/brightfield-shell/tests/${TEST_BINARY}.rs"; do
  [ -f "$SRC/$f" ] || die "the tree has no ${f}"
done

# Every file in the copy gets the time of the copy. `git archive` stamps each
# file with the commit's time and tar keeps the mtimes it is given, and cargo
# decides what to rebuild by comparing a source file's mtime with the build's.
# A target directory kept from an earlier run (BRIGHTFIELD_PROVE_TARGET_DIR)
# that was last built from a newer tree, or from a broken one, then looks newer
# than these sources and answers for them: a run that proves a tree it never
# built. Seen with a chooser broken by hand and left built in that directory:
# the next tree's intact run went red for the leftover, not for its own source.
find "$SRC" -type f -exec touch {} +

# The bytes of each file a break rewrites, to put back byte for byte. After the
# copy back the file is touched: cargo decides what to rebuild from mtimes, and
# a restored file that does not look newer than the build made from the broken
# one lets a stale test binary answer for the intact tree.
cp "$SRC/$TILE_FILE" "$TMP/tile.pristine"
cp "$SRC/$REGISTRY_FILE" "$TMP/registry.pristine"
put_back() {
  cp "$TMP/tile.pristine" "$SRC/$TILE_FILE"
  cp "$TMP/registry.pristine" "$SRC/$REGISTRY_FILE"
  touch "$SRC/$TILE_FILE" "$SRC/$REGISTRY_FILE"
}

# --- the cases --------------------------------------------------------------

echo "== the intact tree"
expect_intact "both structural tests pass"

echo "== one column's recorded kind pinned to another kind"
replace_once "$SRC/$TILE_FILE" \
  $'    Ok(Tile {\n        kind: kind.id,\n        column: column.name.clone(),\n' \
  $'    Ok(Tile {\n        kind: if column.name == "region" { chart_kinds::BINNED_HISTOGRAM } else { kind.id },\n        column: column.name.clone(),\n'
expect_caught "region is recorded as a histogram" "$CHOICE_TEST" \
  '^ *left: .*region: binned-histogram \(from VARCHAR\)' \
  '^ *right: .*region: ranked-category-bars \(from VARCHAR\)'
put_back

echo "== two kinds swapped in the registry's declaration order"
replace_once "$SRC/$REGISTRY_FILE" \
  $'    vec![\n        binned_histogram(),\n        scatter(),\n        point_map(),\n        counts_over_time(),\n' \
  $'    vec![\n        counts_over_time(),\n        scatter(),\n        point_map(),\n        binned_histogram(),\n'
expect_caught "the histogram and the count over time trade places" "$ORDER_TEST" \
  '^ *left: \["counts-over-time", "binned-histogram", "ranked-category-bars"\]' \
  '^ *right: \["binned-histogram", "counts-over-time", "ranked-category-bars"\]'
put_back

echo "== the intact tree again, both breaks put back"
expect_intact "both structural tests pass again"

# --- the checkout -----------------------------------------------------------

echo "== the checkout this was run from"
after=$(checkout_state)
if [ "$before" = "$after" ]; then
  pass "left as it was found"
else
  flunk "the checkout changed during the run"
  diff <(printf '%s\n' "$before") <(printf '%s\n' "$after") | sed 's/^/       /' || true
fi

if [ "$fails" -gt 0 ]; then
  echo "${fails} case(s) failed"
  exit 1
fi
echo "every case held"
