#!/usr/bin/env bash
# Regression test for scripts/check-brew-install.sh — the check's credibility.
#
# The check itself needs a published release and a real Homebrew on a macOS
# runner, so it runs on a tag or a dispatch and nowhere else. That is the shape
# that rots: nothing exercises it between releases, and a release is the worst
# moment to find it stopped reddening. So this file runs the real script against
# a stand-in `brew`, a real throwaway git repository standing in for the tap
# (its casks written by the real generator, scripts/write-brightfield-cask.sh)
# and a stand-in `spctl` and installed binary whose answers the test chooses —
# on a pull request, where a regression is cheap.
#
# Both directions are covered, like the other gate self-tests in this directory:
#
#   1. a healthy install passes, and the log carries the spctl verdict, both
#      installed-copy exit codes and the link's target, so a check that examined
#      nothing cannot look green;
#   2. each check red alone fails, and the failure names the one that was red:
#      Gatekeeper not saying Notarized Developer ID, either flag exiting
#      non-zero, a link to the app's own binary instead of the shim. They are
#      read separately, so none can hide behind another;
#   3. the cask installed is the one the tap wrote for the tag, even when the
#      tap has since moved on or was cloned shallow, and a tag the tap holds no
#      cask for is a hard failure that installs nothing;
#   4. an install that fails, or that puts a different version on the machine,
#      fails before any check runs, so the checks never read some other copy;
#   5. usage errors, a missing brew, a tap that will not tap, and a run outside
#      CI are hard failures rather than passes;
#   6. release.yml runs the check after the cask is pushed and passes it the
#      release's tag, and brew-install.yml can be both called and dispatched —
#      the wiring that puts the check's failure in the release run.
#
# Usage:  ./scripts/check-brew-install-selftest.sh
# Exit:   0 all assertions passed, 1 otherwise.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
CHECK="$HERE/check-brew-install.sh"
GENERATOR="$HERE/write-brightfield-cask.sh"
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
# claims v0.1.7 while carrying v0.1.6's cask. The tip is not the newest
# honest release on purpose: every case below has to reach its cask through
# the tag, not through wherever the checkout happens to be.
# ---------------------------------------------------------------------------
SHA_A="$(printf '%064d' 1)"
SHA_B="$(printf '%064d' 2)"

gitq() { git -C "$TAPDIR" -c user.name=fixture -c user.email=fixture@example.invalid -c commit.gpgsign=false "$@"; }

write_cask() {
	"$GENERATOR" "$1" "https://example.invalid/releases/download/$1" "$SHA_A" "$SHA_B" \
		>"$TAPDIR/Casks/brightfield.rb" || {
		echo "selftest: the cask generator failed for $1" >&2
		exit 1
	}
}

mkdir -p "$TAPDIR/Casks" "$TAPDIR/Formula"
git -C "$TAPDIR" init -q
git -C "$TAPDIR" checkout -q -b main

write_cask v0.1.4
gitq add Casks && gitq commit -q -m "Update brightfield to v0.1.4"
write_cask v0.1.5
gitq add Casks && gitq commit -q -m "Update brightfield to v0.1.5"
printf 'class Finetype < Formula\nend\n' >"$TAPDIR/Formula/finetype.rb"
gitq add Formula && gitq commit -q -m "Update finetype to v9.9.9"
write_cask v0.1.6
gitq add Casks && gitq commit -q -m "Update brightfield to v0.1.6"
write_cask v0.1.6
printf '# touched\n' >>"$TAPDIR/Casks/brightfield.rb"
gitq add Casks && gitq commit -q -m "Update brightfield to v0.1.7"

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
	[[ "$2" == "--cask" ]] || { echo "fixture brew: install without --cask: $*" >&2; exit 98; }
	appdir="$FAKE_DEFAULT_APPDIR"
	for ((i = 1; i < $#; i++)); do
		[[ "${!i}" == "--appdir" ]] && { j=$((i + 1)); appdir="${!j}"; }
	done
	v="$(sed -n 's/^  version "\(.*\)"$/\1/p' "$FAKE_TAPDIR/Casks/brightfield.rb" | head -1)"
	mkdir -p "$FAKE_PREFIX/bin" "$FAKE_PREFIX/Caskroom/brightfield/$v" "$appdir/Brightfield.app/Contents/MacOS"
	printf '%s\n' "${FAKE_INSTALLED_VERSION:-$v}" >"$FAKE_PREFIX/installed-version"
	cp "$FAKE_BIN_TEMPLATE" "$appdir/Brightfield.app/Contents/MacOS/brightfield"
	cp "$FAKE_BIN_TEMPLATE" "$FAKE_PREFIX/Caskroom/brightfield/$v/brightfield.wrapper.sh"
	chmod +x "$appdir/Brightfield.app/Contents/MacOS/brightfield" "$FAKE_PREFIX/Caskroom/brightfield/$v/brightfield.wrapper.sh"
	if [[ -n "${FAKE_LINK_STALE:-}" ]]; then
		mkdir -p "$FAKE_PREFIX/Caskroom/brightfield/0.0.1"
		cp "$FAKE_BIN_TEMPLATE" "$FAKE_PREFIX/Caskroom/brightfield/0.0.1/brightfield.wrapper.sh"
		ln -s "$FAKE_PREFIX/Caskroom/brightfield/0.0.1/brightfield.wrapper.sh" "$FAKE_PREFIX/bin/brightfield"
	elif [[ -n "${FAKE_LINK_TO_APP:-}" ]]; then
		ln -s "$appdir/Brightfield.app/Contents/MacOS/brightfield" "$FAKE_PREFIX/bin/brightfield"
	else
		ln -s "$FAKE_PREFIX/Caskroom/brightfield/$v/brightfield.wrapper.sh" "$FAKE_PREFIX/bin/brightfield"
	fi
	echo "fixture: installed brightfield $(cat "$FAKE_PREFIX/installed-version")"
	;;
list)
	[[ "$2 $3" == "--cask --versions" ]] || { echo "fixture brew: list without --cask --versions: $*" >&2; exit 98; }
	echo "brightfield $(cat "$FAKE_PREFIX/installed-version")"
	;;
*) echo "fixture brew: unexpected: $*" >&2; exit 99 ;;
esac
BREW
chmod +x "$SHIM/brew"

# `spctl` answers on stderr, as the real one does. The source line is the one
# the check reads.
cat >"$SHIM/spctl" <<'SPCTL'
#!/usr/bin/env bash
printf 'spctl %s\n' "$*" >>"$FAKE_CALLS"
{
	echo "${!#}: accepted"
	echo "source=${FAKE_SPCTL_SOURCE-Notarized Developer ID}"
	echo "origin=Developer ID Application: Fixture (ABCDE12345)"
} >&2
exit "${FAKE_SPCTL_RC:-0}"
SPCTL
chmod +x "$SHIM/spctl"

cat >"$TMP/installed-brightfield" <<'BIN'
#!/usr/bin/env bash
case "$1" in
--check-type-source)
	[[ -n "${FAKE_LAUNCH_SLEEP:-}" ]] && sleep "$FAKE_LAUNCH_SLEEP"
	echo "fixture-installed-check: ${FAKE_CHECK_MSG:-came up}"
	exit "${FAKE_CHECK_RC:-0}"
	;;
--check-engine)
	echo "fixture-installed-engine: ${FAKE_ENGINE_MSG:-came up}"
	exit "${FAKE_ENGINE_RC:-0}"
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
	rm -rf "$PREFIX" "$TMP/default-apps" "$TMP/apps"
	mkdir -p "$PREFIX"
	: >"$CALLS"
	env CI=true PATH="$SHIM:$PATH" \
		FAKE_CALLS="$CALLS" FAKE_TAPDIR="$TAPDIR" FAKE_PREFIX="$PREFIX" \
		FAKE_BIN_TEMPLATE="$TMP/installed-brightfield" FAKE_DEFAULT_APPDIR="$TMP/default-apps" \
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
run 0 "installs the tag and every check passes" -- --tag v0.1.6
has "the install is a cask install" "brew install --cask meridian-online/tap/brightfield"
has "the log carries Gatekeeper's verdict" "source=Notarized Developer ID"
has "the log carries the installed copy's check output" "fixture-installed-check: came up"
has "the log carries that check's exit code" "brightfield --check-type-source exit=0"
has "the log carries the engine check's exit code" "brightfield --check-engine exit=0"
has "the log carries the link's target" "brightfield.wrapper.sh"
has "the log names the version that was installed" "installed: brightfield 0.1.6"
has "spctl read the app in the default application directory" "spctl -a -vv -t exec /Applications/Brightfield.app"
lacks "no brew test is run: casks have none" "brew test"

run 0 "--appdir reaches the install and the Gatekeeper check" -- --tag v0.1.6 --appdir "$TMP/apps"
has "brew installs into that directory" "--appdir $TMP/apps"
has "spctl reads the app there" "spctl -a -vv -t exec $TMP/apps/Brightfield.app"

# From inside $TMP, so the stand-in install lands there and not in the checkout.
cd "$TMP" || exit 1
run 0 "a relative --appdir is made absolute" -- --tag v0.1.6 --appdir rel-apps
cd "$ROOT" || exit 1
has "brew installs into the absolute path" "--appdir $TMP/rel-apps"
has "spctl reads the app there" "spctl -a -vv -t exec $TMP/rel-apps/Brightfield.app"

# ---------------------------------------------------------------------------
# 2. The fault, and each check red on its own.
# ---------------------------------------------------------------------------
echo
echo "the installed copy fails"
run 1 "every check red at once" FAKE_SPCTL_SOURCE="Unnotarized Developer ID" FAKE_CHECK_RC=1 \
	"FAKE_CHECK_MSG=$REFUSAL" FAKE_ENGINE_RC=1 FAKE_LINK_TO_APP=1 -- --tag v0.1.6
has "the log carries the refusal from the installed copy" "fixture-installed-check: $REFUSAL"
has "the log names Gatekeeper" "Gatekeeper does not accept"
has "the log names the type-source check" "--check-type-source\` exited 1"
has "the log names the engine check" "--check-engine\` exited 1"
has "the log names the link" "not the cask's exec shim"

run 1 "Gatekeeper not saying Notarized Developer ID, the rest green" FAKE_SPCTL_SOURCE="Developer ID" -- --tag v0.1.6
has "the failure names Gatekeeper" "Gatekeeper does not accept"
lacks "and does not blame the type-source check" "--check-type-source\` exited"
lacks "and does not blame the engine check" "--check-engine\` exited"
lacks "and does not blame the link" "exec shim"

run 1 "an ad hoc app: spctl says nothing of a source" FAKE_SPCTL_SOURCE= FAKE_SPCTL_RC=3 -- --tag v0.1.6
has "the failure names Gatekeeper" "Gatekeeper does not accept"

run 1 "the installed type-source check red alone" FAKE_CHECK_RC=2 "FAKE_CHECK_MSG=$REFUSAL" -- --tag v0.1.6
has "the failure names the type-source check" "--check-type-source\` exited 2"
lacks "and does not blame Gatekeeper" "Gatekeeper does not accept"
lacks "and does not blame the engine check" "--check-engine\` exited"

run 1 "the engine check red alone" FAKE_ENGINE_RC=1 -- --tag v0.1.6
has "the failure names the engine check" "--check-engine\` exited 1"
lacks "and does not blame the type-source check" "--check-type-source\` exited"

run 1 "a launch that never returns is cut off and named" FAKE_LAUNCH_SLEEP=30 BRIGHTFIELD_LAUNCH_LIMIT_SECONDS=1 -- --tag v0.1.6
has "the failure names the launch as blocked" "the launch is blocked, most likely waiting on Gatekeeper"
has "and names the launch" "brightfield --check-type-source\` was still running after 1 s"

run 1 "a link to the app's own binary, the rest green" FAKE_LINK_TO_APP=1 -- --tag v0.1.6
has "the failure names the link" "not the cask's exec shim"
lacks "and does not blame Gatekeeper" "Gatekeeper does not accept"

run 1 "a link to another version's shim, the rest green" FAKE_LINK_STALE=1 -- --tag v0.1.6
has "the failure names the link" "not the cask's exec shim"
lacks "and does not blame Gatekeeper" "Gatekeeper does not accept"

run 1 "spctl names the source but exits non-zero" FAKE_SPCTL_RC=3 -- --tag v0.1.6
has "the failure names Gatekeeper" "Gatekeeper does not accept"
has "the exit code is in the failure" "spctl exit 3"

# ---------------------------------------------------------------------------
# 3. Which cask is installed.
# ---------------------------------------------------------------------------
echo
echo "which cask is installed"
run 0 "an older tag installs the cask the tap wrote for it, not the tap's tip" -- --tag v0.1.5
has "the version installed is the older tag's" "installed: brightfield 0.1.5"
has "the log names the tap commit the cask came from" "cask from meridian-online/tap commit"

# A shallow clone holds the tip and nothing older, so an older tag's commit is
# there only after the script deepens it.
# Cut from the tap's branch tip, not from wherever an earlier case left the
# checkout: a clone that already holds the commit needs no deepening.
rm -rf "$TMP/shallow"
git -C "$TAPDIR" checkout -q main
git clone -q --depth 1 "file://$TAPDIR" "$TMP/shallow"
run 0 "a shallow tap checkout is deepened to find an older tag's cask" "FAKE_TAPDIR=$TMP/shallow" -- --tag v0.1.5
has "the version installed is the older tag's" "installed: brightfield 0.1.5"

run 2 "a tag the tap holds no cask for" -- --tag v0.2.0
has "the message says the tap holds none" "holds no cask written for v0.2.0"
not_called "and nothing was installed" "install"

run 2 "a prefix of a released tag is not that release" -- --tag v0.1
has "the message says the tap holds none" "holds no cask written for v0.1 "
not_called "and nothing was installed" "install"

run 2 "a commit titled for the tag whose file addresses another release" -- --tag v0.1.7
has "the message says the file does not address the tag" "does not address v0.1.7"
not_called "and nothing was installed" "install"

# ---------------------------------------------------------------------------
# 4. The install itself.
# ---------------------------------------------------------------------------
echo
echo "the install itself"
run 1 "an install that fails" FAKE_INSTALL_RC=1 -- --tag v0.1.6
has "the message says the cask did not install" "did not install"
not_called "no check ran" "list"
if grep -Fq "spctl" "$CALLS"; then bad "spctl ran after a failed install"; else ok "spctl did not run"; fi

run 1 "an install that puts a different version on the machine" FAKE_INSTALLED_VERSION=0.0.9 -- --tag v0.1.6
has "the message names the version found" "brightfield 0.0.9"
if grep -Fq "spctl" "$CALLS"; then bad "spctl ran against a different version"; else ok "spctl did not run"; fi

# ---------------------------------------------------------------------------
# 5. The check that cannot run.
# ---------------------------------------------------------------------------
echo
echo "the check cannot run"
# Each refusal is pinned by its message as well as its exit code: exit 2 is also
# what a tag the tap has no cask for returns, so a refusal that stopped firing
# could still exit 2 further down and look the same.
run 2 "no --tag" -- --allow-local
has "the refusal names the tag" "--tag must look like v1.2.3"
not_called "and brew was not touched" "--version"
run 2 "a --tag with no value" -- --tag
has "the refusal names the missing value" "--tag needs a value"
run 2 "a tag that is not a version" -- --tag 'v1.2.3; echo pwned'
has "the refusal names the tag" "--tag must look like v1.2.3"
not_called "and brew was not touched" "--version"
run 2 "a tag without the v" -- --tag 0.1.6
has "the refusal names the tag" "--tag must look like v1.2.3"
run 2 "an unknown argument" -- --tag v0.1.6 --nope
has "the refusal names the argument" "unknown argument: --nope"
run 2 "a tap that is not OWNER/NAME" -- --tag v0.1.6 --tap 'not a tap'
has "the refusal names the tap" "--tap must look like OWNER/NAME"
run 2 "a tap that will not tap" FAKE_TAP_RC=1 -- --tag v0.1.6
has "the refusal says the tap would not tap" "could not tap meridian-online/tap"
not_called "and nothing was installed" "install"
run 2 "no brew on PATH" PATH=/usr/bin:/bin -- --tag v0.1.6
has "the refusal says brew is missing" "brew is not on PATH"
run 2 "outside CI" CI=false -- --tag v0.1.6
has "the refusal says why" "refusing to run outside CI"
not_called "and brew was not touched" "--version"
run 0 "outside CI, when asked for" CI=false -- --tag v0.1.6 --allow-local

# ---------------------------------------------------------------------------
# 6. The wiring that puts the check's failure in the release run.
#
# Read from the workflow files, because there is no runner here to run them and
# a tag is the only thing that runs release.yml. What is pinned is what makes the
# check part of the release: a job that calls brew-install.yml, after the job
# that pushes the cask, with the release's own tag.
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
