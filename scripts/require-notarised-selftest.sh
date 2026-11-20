#!/usr/bin/env bash
# Prove scripts/require-notarised.sh refuses anything but `notarised`, and that
# release.yml's update-homebrew job runs it before it touches the tap.
#
# A refusal nobody calls is no refusal: the wiring cases read release.yml and
# fail when the job's first step is not this script fed the build's output, or
# when that step can be skipped or have its failure ignored.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
CHECK="$HERE/require-notarised.sh"
WORKFLOW="$ROOT/.github/workflows/release.yml"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
out="$TMP/out"

expect() { # expect NAME EXIT VALUE
	"$CHECK" "$3" >"$out" 2>&1
	local rc=$?
	if [[ "$rc" -ne "$2" ]]; then
		echo "  FAIL $1: expected exit $2, got ${rc}"
		failures=$((failures + 1))
	elif [[ "$2" -eq 1 ]] && ! grep -q '^::error::' "$out"; then
		echo "  FAIL $1: refused without an ::error::"
		failures=$((failures + 1))
	else
		echo "  ok   $1"
	fi
}

echo "== the values"
expect "notarised passes" 0 notarised
expect "adhoc is refused" 1 adhoc
expect "an empty value is refused" 1 ""
expect "Notarised is refused" 1 Notarised
"$CHECK" adhoc >"$out" 2>&1
if grep -q "'adhoc'" "$out"; then
	echo "  ok   the refusal names the value"
else
	echo "  FAIL the refusal does not name the value"
	failures=$((failures + 1))
fi
"$CHECK" >"$out" 2>&1
[[ $? -eq 1 ]] && echo "  ok   no argument is refused" || { echo "  FAIL no argument was not refused"; failures=$((failures + 1)); }

echo "== release.yml runs it ahead of everything but the checkout"
# The job's steps, as text, split on the `      - ` that opens each one. The
# scripts live in this repository, so the checkout of it has to come first; that
# is the one step allowed ahead of the refusal, and it touches nothing. Done in
# Python so the YAML's own indentation is the rule.
python3 - "$WORKFLOW" <<'PY'
import re, sys

text = open(sys.argv[1], encoding="utf-8").read()
m = re.search(r"^  update-homebrew:\n(.*?)(?=^  \S|\Z)", text, re.M | re.S)
if not m:
    print("  FAIL release.yml has no update-homebrew job")
    sys.exit(1)
job = m.group(1)
steps = re.search(r"^    steps:\n(.*)", job, re.M | re.S)
if not steps:
    print("  FAIL update-homebrew has no steps")
    sys.exit(1)
body = re.sub(r"^ *#.*\n", "", steps.group(1), flags=re.M)
parts = re.split(r"^      - ", body, flags=re.M)[1:]
checkout = parts[0]
first = parts[1] if len(parts) > 1 else ""
bad = 0
def need(ok, what):
    global bad
    print(("  ok   " if ok else "  FAIL ") + what)
    bad += not ok
need(checkout.strip() == "uses: actions/checkout@v4", "only a plain checkout of this repository comes before it")
need('scripts/require-notarised.sh "$SIGNING"' in first, "the step after the checkout runs require-notarised.sh with $SIGNING")
need(re.search(r"^\s+SIGNING: \$\{\{ needs\.build\.outputs\.signing \}\}\s*$", first, re.M) is not None
     and re.search(r"^\s+env:\s*$", first, re.M) is not None,
     "SIGNING comes from the build's signing output through env:")
need(re.search(r"^\s+(if|continue-on-error):", first, re.M) is None, "that step has no if: or continue-on-error:")
need(re.search(r"^    (if|continue-on-error):", job, re.M) is None, "the job has no if: or continue-on-error:")
need(re.search(r"^    needs:.*\bbuild\b", job, re.M) is not None, "the job needs build, whose output it reads")
sys.exit(1 if bad else 0)
PY
[[ $? -eq 0 ]] || failures=$((failures + 1))

echo
if [[ "$failures" -ne 0 ]]; then
	echo "require-notarised-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "require-notarised-selftest: only a notarised build passes, and the release job runs the check first."
