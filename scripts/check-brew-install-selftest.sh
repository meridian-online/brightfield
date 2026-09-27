#!/usr/bin/env bash
# Regression test for scripts/check-brew-install.sh — the check's credibility.
#
# The check itself needs a published release and a real Homebrew on a macOS
# runner, so it runs on a tag or a dispatch and nowhere else. That is the shape
# that rots: nothing exercises it between releases, and a release is the worst
# moment to find it stopped reddening. So this file runs the real script against
# a stand-in `brew`, a real throwaway git repository standing in for the tap
# (its formulae written by the real generator, scripts/write-brightfield-formula.sh)
# and a stand-in installed binary whose answers the test chooses — on a pull
# request, where a regression is cheap.
#
# Both directions are covered, like the other gate self-tests in this directory:
#
#   1. a healthy install passes, and the log carries the exit code of
#      `brew test` and the output and exit code of the installed copy's
#      `--check-type-source`, so a check that examined nothing cannot look green;
#   2. the fault the check exists for — `brew test` red and the installed copy
#      refusing its type source — fails, and the log names both;
#   3. either run red alone fails, and the failure names the run that was red:
#      the two are read separately, so neither can hide behind the other;
#   4. the formula installed is the one the tap wrote for the tag, even when the
#      tap has since moved on, and a tag the tap holds no formula for is a hard
#      failure that installs nothing;
#   5. an install that fails, or that puts a different version on the machine,
#      fails before `brew test` runs, so the two runs never read some other copy;
#   6. usage errors, a missing brew, a tap that will not tap, and a run outside
#      CI are hard failures rather than passes;
#   7. release.yml runs the check after the formula is pushed and passes it the
#      release's tag, and brew-install.yml can be both called and dispatched —
#      the wiring that puts the check's failure in the release run.
#
# Usage:  ./scripts/check-brew-install-selftest.sh
# Exit:   0 all assertions passed, 1 otherwise.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
CHECK="$HERE/check-brew-install.sh"
GENERATOR="$HERE/write-brightfield-formula.sh"
RELEASE_YML="$ROOT/.github/workflows/release.yml"
BREW_YML="$ROOT/.github/workflows/brew-install.yml"

for f in "$CHECK" "$GENERATOR"; do
	[[ -x "$f" ]] || {
		echo "selftest: $f is missing or not executable" >&2
		exit 1
	}
done
command -v git >/dev/null 2>&1 || {
	echo "selftest: git is required to build the fixture tap" >&2
	exit 1
}

failures=0
ok() { printf '  ok    %s\n' "$1"; }
bad() {
	printf '  FAIL  %s\n' "$1"
	failures=$((failures + 1))
}

TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT

TAPDIR="$TMP/tap"
PREFIX="$TMP/prefix"
SHIM="$TMP/shim"
CALLS="$TMP/calls"
OUT="$TMP/out"
mkdir -p "$SHIM"

# ---------------------------------------------------------------------------
# The stand-in tap: a git repository with the history the update-homebrew job
# leaves behind — one commit per release titled "Update brightfield to <tag>",
# an unrelated formula's commit between them, and at the tip a commit that
# claims v0.1.7 while carrying v0.1.6's formula. The tip is not the newest
# honest release on purpose: every case below has to reach its formula through
# the tag, not through wherever the checkout happens to be.
# ---------------------------------------------------------------------------
SHA_A="$(printf '%064d' 1)"
SHA_B="$(printf '%064d' 2)"

gitq() { git -C "$TAPDIR" -c user.name=fixture -c user.email=fixture@example.invalid -c commit.gpgsign=false "$@"; }

write_formula() {
	"$GENERATOR" "$1" "https://example.invalid/releases/download/$1" "$SHA_A" "$SHA_B" \
		>"$TAPDIR/Formula/brightfield.rb" || {
		echo "selftest: the formula generator failed for $1" >&2
		exit 1
	}
}

mkdir -p "$TAPDIR/Formula"
git -C "$TAPDIR" init -q
git -C "$TAPDIR" checkout -q -b main

write_formula v0.1.4
gitq add Formula && gitq commit -q -m "Update brightfield to v0.1.4"
write_formula v0.1.5
gitq add Formula && gitq commit -q -m "Update brightfield to v0.1.5"
printf 'class Finetype < Formula\nend\n' >"$TAPDIR/Formula/finetype.rb"
gitq add Formula && gitq commit -q -m "Update finetype to v9.9.9"
write_formula v0.1.6
gitq add Formula && gitq commit -q -m "Update brightfield to v0.1.6"
write_formula v0.1.6
printf '# touched\n' >>"$TAPDIR/Formula/brightfield.rb"
gitq add Formula && gitq commit -q -m "Update brightfield to v0.1.7"

# ---------------------------------------------------------------------------
# The stand-in `brew` and the stand-in installed binary. What `brew install`
# "installs" is read from the tap's working tree at the moment it runs, so a
# check that installed from the wrong commit installs the wrong version here.
# ---------------------------------------------------------------------------
cat >"$SHIM/brew" <<'BREW'
#!/usr/bin/env bash
printf 'brew %s\n' "$*" >>"$FAKE_CALLS"
case "$1" in
--version) echo "Homebrew 0.0.0-fixture" ;;
tap) exit "${FAKE_TAP_RC:-0}" ;;
--repository) printf '%s\n' "$FAKE_TAPDIR" ;;
--prefix) printf '%s\n' "$FAKE_PREFIX" ;;
install)
	if [[ "${FAKE_INSTALL_RC:-0}" != 0 ]]; then
		echo "Error: fixture install failure"
		exit "$FAKE_INSTALL_RC"
	fi
	v="$(grep -o 'brightfield-v[0-9][0-9.]*-aarch64' "$FAKE_TAPDIR/Formula/brightfield.rb" | head -1)"
	v="${v#brightfield-v}"
	v="${v%-aarch64}"
	mkdir -p "$FAKE_PREFIX/bin"
	printf '%s\n' "${FAKE_INSTALLED_VERSION:-$v}" >"$FAKE_PREFIX/installed-version"
	cp "$FAKE_BIN_TEMPLATE" "$FAKE_PREFIX/bin/brightfield"
	chmod +x "$FAKE_PREFIX/bin/brightfield"
	echo "fixture: installed brightfield $(cat "$FAKE_PREFIX/installed-version")"
	;;
test)
	echo "fixture-brew-test: ran"
	[[ "${FAKE_TEST_RC:-0}" == 0 ]] || echo "Error: brightfield: failed"
	exit "${FAKE_TEST_RC:-0}"
	;;
list) echo "brightfield $(cat "$FAKE_PREFIX/installed-version")" ;;
*) echo "fixture brew: unexpected: $*" >&2; exit 99 ;;
esac
BREW
chmod +x "$SHIM/brew"

cat >"$TMP/installed-brightfield" <<'BIN'
#!/usr/bin/env bash
case "$1" in
--check-type-source)
	echo "fixture-installed-check: ${FAKE_CHECK_MSG:-came up}"
	exit "${FAKE_CHECK_RC:-0}"
	;;
esac
exit 64
BIN

# ---------------------------------------------------------------------------
# run <want rc> <label> [NAME=value ...] -- <check args...>
#
# Each run starts from the same place: the tap on its branch, an empty prefix
# and an empty call log. CI=true is the default; a case overrides it or PATH by
# naming them, and the later assignment wins.
# ---------------------------------------------------------------------------
run() {
	local want="$1" label="$2"
	shift 2
	local -a envs=()
	while [[ $# -gt 0 && "$1" != "--" ]]; do
		envs+=("$1")
		shift
	done
	shift
	git -C "$TAPDIR" checkout -q main
	rm -rf "$PREFIX"
	mkdir -p "$PREFIX"
	: >"$CALLS"
	env CI=true PATH="$SHIM:$PATH" \
		FAKE_CALLS="$CALLS" FAKE_TAPDIR="$TAPDIR" FAKE_PREFIX="$PREFIX" \
		FAKE_BIN_TEMPLATE="$TMP/installed-brightfield" \
		${envs[@]+"${envs[@]}"} \
		"$BASH" "$CHECK" "$@" >"$OUT" 2>&1
	local got=$?
	if [[ $got -eq $want ]]; then
		ok "$label — exit $got"
	else
		bad "$label — expected exit $want, got $got"
		sed 's/^/        | /' "$OUT"
	fi
}

# `has <label> <text>` — the last run's output carries the text, verbatim.
has() {
	if grep -Fq -- "$2" "$OUT"; then
		ok "$1"
	else
		bad "$1 — the output does not carry: $2"
		sed 's/^/        | /' "$OUT"
	fi
}
# `lacks <label> <text>` — it does not.
lacks() {
	if grep -Fq -- "$2" "$OUT"; then
		bad "$1 — the output carries: $2"
		sed 's/^/        | /' "$OUT"
	else
		ok "$1"
	fi
}
# `called` / `not_called` — whether the stand-in brew was asked for a subcommand.
called() {
	if grep -Fq -- "brew $2" "$CALLS"; then ok "$1"; else bad "$1 — brew was never asked for: $2"; fi
}
not_called() {
	if grep -Fq -- "brew $2" "$CALLS"; then bad "$1 — brew was asked for: $2"; else ok "$1"; fi
}

REFUSAL="finetype.duckdb_extension is not the file that was packaged"

# ---------------------------------------------------------------------------
# 1. A healthy install.
# ---------------------------------------------------------------------------
echo "a healthy install"
run 0 "installs the tag and both runs pass" -- --tag v0.1.6
has "the log carries brew test's exit code" "brew test brightfield exit=0"
has "the log carries the installed copy's check output" "fixture-installed-check: came up"
has "the log carries that check's exit code" "brightfield --check-type-source exit=0"
has "the log names the version that was installed" "installed: brightfield 0.1.6"

# ---------------------------------------------------------------------------
# 2 and 3. The fault, and each run red on its own.
# ---------------------------------------------------------------------------
echo
echo "the installed copy fails"
run 1 "brew test red and the installed check refusing its type source" \
	FAKE_TEST_RC=1 FAKE_CHECK_RC=1 "FAKE_CHECK_MSG=$REFUSAL" -- --tag v0.1.6
has "the log carries brew test's exit code" "brew test brightfield exit=1"
has "the log carries the refusal from the installed copy" "fixture-installed-check: $REFUSAL"
has "the log carries the installed check's exit code" "brightfield --check-type-source exit=1"

run 1 "brew test green, the installed copy's own check red" \
	FAKE_CHECK_RC=1 "FAKE_CHECK_MSG=$REFUSAL" -- --tag v0.1.6
has "the failure names the installed check" "--check-type-source\` exited 1"
lacks "and does not blame brew test" "brew test brightfield\` exited"

run 1 "brew test red, the installed copy's own check green" FAKE_TEST_RC=1 -- --tag v0.1.6
has "the failure names brew test" "brew test brightfield\` exited 1"
lacks "and does not blame the installed check" "--check-type-source\` exited"

# ---------------------------------------------------------------------------
# 4. Which formula is installed.
# ---------------------------------------------------------------------------
echo
echo "which formula is installed"
run 0 "an older tag installs the formula the tap wrote for it, not the tap's tip" -- --tag v0.1.5
has "the version installed is the older tag's" "installed: brightfield 0.1.5"
has "the log names the tap commit the formula came from" "formula from meridian-online/tap commit"

run 2 "a tag the tap holds no formula for" -- --tag v0.2.0
has "the message says the tap holds none" "holds no formula written for v0.2.0"
not_called "and nothing was installed" "install"

run 2 "a prefix of a released tag is not that release" -- --tag v0.1
has "the message says the tap holds none" "holds no formula written for v0.1 "
not_called "and nothing was installed" "install"

run 2 "a commit titled for the tag whose file addresses another release" -- --tag v0.1.7
has "the message says the file does not address the tag" "does not address v0.1.7"
not_called "and nothing was installed" "install"

# ---------------------------------------------------------------------------
# 5. The install itself.
# ---------------------------------------------------------------------------
echo
echo "the install itself"
run 1 "an install that fails" FAKE_INSTALL_RC=1 -- --tag v0.1.6
not_called "brew test did not run" "test"

run 1 "an install that puts a different version on the machine" FAKE_INSTALLED_VERSION=0.0.9 -- --tag v0.1.6
has "the message names the version found" "brightfield 0.0.9"
not_called "brew test did not run" "test"

# ---------------------------------------------------------------------------
# 6. The check that cannot run.
# ---------------------------------------------------------------------------
echo
echo "the check cannot run"
run 2 "no --tag" -- --allow-local
run 2 "a --tag with no value" -- --tag
run 2 "a tag that is not a version" -- --tag 'v1.2.3; echo pwned'
run 2 "a tag without the v" -- --tag 0.1.6
run 2 "an unknown argument" -- --tag v0.1.6 --nope
run 2 "a tap that is not OWNER/NAME" -- --tag v0.1.6 --tap 'not a tap'
run 2 "a tap that will not tap" FAKE_TAP_RC=1 -- --tag v0.1.6
not_called "and nothing was installed" "install"
run 2 "no brew on PATH" PATH=/usr/bin:/bin -- --tag v0.1.6
run 2 "outside CI" CI=false -- --tag v0.1.6
not_called "and brew was not touched" "--version"
run 0 "outside CI, when asked for" CI=false -- --tag v0.1.6 --allow-local

# ---------------------------------------------------------------------------
# 7. The wiring that puts the check's failure in the release run.
#
# Read from the workflow files, because there is no runner here to run them and
# a tag is the only thing that runs release.yml. What is pinned is what makes the
# check part of the release: a job that calls brew-install.yml, after the job
# that pushes the formula, with the release's own tag.
# ---------------------------------------------------------------------------
echo
echo "the workflow wiring"

job_block() {
	# The lines of the top-level job `$2` in workflow `$1`, up to the next job.
	awk -v job="  $2:" '
		$0 == job { on = 1; print; next }
		on && /^  [A-Za-z0-9_-]+:/ { exit }
		on { print }
	' "$1"
}

has_line() {
	# `has_line <label> <block> <exact line>`
	if printf '%s\n' "$2" | grep -Fxq -- "$3"; then ok "$1"; else bad "$1 — no line: $3"; fi
}

caller="$(job_block "$RELEASE_YML" brew-install)"
if [[ -z "$caller" ]]; then
	bad "release.yml declares no brew-install job"
else
	has_line "release.yml's brew-install job runs after update-homebrew" "$caller" "    needs: update-homebrew"
	has_line "it calls the reusable workflow" "$caller" "    uses: ./.github/workflows/brew-install.yml"
	has_line "it passes the release's own tag" "$caller" '      tag: ${{ github.ref_name }}'
fi

if grep -Fxq "  workflow_call:" "$BREW_YML" && grep -Fxq "  workflow_dispatch:" "$BREW_YML"; then
	ok "brew-install.yml can be called and dispatched"
else
	bad "brew-install.yml is missing workflow_call or workflow_dispatch"
fi

runner="$(job_block "$BREW_YML" brew-install)"
has_line "it runs on the hosted arm64 macOS image" "$runner" "    runs-on: macos-15"
if printf '%s\n' "$runner" | grep -Fq 'scripts/check-brew-install.sh --tag "$TAG"'; then
	ok "it runs the check with the tag it was given"
else
	bad "brew-install.yml does not run scripts/check-brew-install.sh --tag \"\$TAG\""
fi

echo
if [[ $failures -ne 0 ]]; then
	echo "check-brew-install self-test: $failures assertion(s) FAILED" >&2
	exit 1
fi
echo "check-brew-install self-test: ok"
