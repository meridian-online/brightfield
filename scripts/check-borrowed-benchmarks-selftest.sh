#!/usr/bin/env bash
# Regression test for check-borrowed-benchmarks.sh: the gate has to keep
# matching the inherited figures AND keep ignoring honest prose, and both
# failure modes are silent — so this runs first in CI, exactly like the
# public-hygiene gate's self-test.
#
# The gate reads tracked files via `git grep`, so the fixtures live in a
# throwaway git repository under mktemp, with a copy of the gate script.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
GATE="$PWD/scripts/check-borrowed-benchmarks.sh"

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git -C "$tmp" init -q
mkdir -p "$tmp/scripts"
cp "$GATE" "$tmp/scripts/"

must_fail=(
  'renders interactively at 60+ FPS on anything'
  'sub-100ms filter response times'
  'sub-100 ms filter response'
  '0.01s interaction latency at scale'
  '0.01 seconds latency at scale'
  'real-time interaction with billion-row datasets'
  'billion-element databases at interactive rates'
)
must_pass=(
  'the goal is datasets from thousands to billions of records'
  'the animation runs at 60 fps on a 60 Hz panel'
  'a 100ms budget for the whole frame'
  'the timer fired after 0.011s'
  'billions of rows is the aspiration'
)

run_gate() {
  ( cd "$tmp" && ./scripts/check-borrowed-benchmarks.sh >/dev/null 2>&1 )
}

fixture() {
  printf '%s\n' "$1" > "$tmp/doc.md"
  git -C "$tmp" add -A
  git -C "$tmp" -c user.email=t@t -c user.name=t commit -qm fixture --allow-empty
}

for s in "${must_fail[@]}"; do
  fixture "$s"
  if run_gate; then
    echo "SELF-TEST FAILED: gate stayed silent on: $s" >&2
    exit 1
  fi
done

for s in "${must_pass[@]}"; do
  fixture "$s"
  if ! run_gate; then
    echo "SELF-TEST FAILED: gate cried wolf on: $s" >&2
    exit 1
  fi
done

# A reason-less allowlist entry is a malformed gate, never a pass.
fixture 'renders interactively at 60+ FPS on anything'
printf '%s\n' 'doc.md' > "$tmp/scripts/borrowed-benchmarks-allowlist.txt"
git -C "$tmp" add -A
git -C "$tmp" -c user.email=t@t -c user.name=t commit -qm allowlist
if run_gate; then
  echo "SELF-TEST FAILED: reason-less allowlist entry was accepted" >&2
  exit 1
fi

# A well-formed allowlist entry silences exactly its file.
printf 'doc.md\tmeasured here, cited with its machine record\n' > "$tmp/scripts/borrowed-benchmarks-allowlist.txt"
git -C "$tmp" add -A
git -C "$tmp" -c user.email=t@t -c user.name=t commit -qm allowlist2
if ! run_gate; then
  echo "SELF-TEST FAILED: allowlisted file still failed the gate" >&2
  exit 1
fi

echo "borrowed-benchmarks gate self-test: ok"
