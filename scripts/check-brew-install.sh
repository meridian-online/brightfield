#!/usr/bin/env bash
# Install the Homebrew formula the tap holds for a published tag, then run the
# copy Homebrew installed — through the formula's own test, and directly.
#
#   scripts/check-brew-install.sh --tag vX.Y.Z [--tap OWNER/NAME] [--allow-local]
#
# WHY THIS EXISTS. The release's other checks read the package BEFORE Homebrew
# has touched it: package.sh builds the tarball, the type-source check reads the
# binary inside it, and scripts/check-formula-asset.sh hashes the asset the
# formula addresses. `brew install` then unpacks that tarball into the Cellar,
# and the Cellar copy is the one a person runs. No check read that copy, so a
# fault that exists after Homebrew's install reached a person's machine before it
# reached anyone here, and the release run that published it was green.
#
# WHAT IT DOES, in order:
#
#   1. Taps the tap and moves its checkout to the commit that wrote the formula
#      for --tag (the commit titled "Update brightfield to <tag>", which is what
#      the update-homebrew job in release.yml commits). In a release run that is
#      the commit the previous job just pushed. Dispatched by hand for an older
#      tag, it is the formula the tap held for that tag, not whatever the tap
#      holds now.
#   2. `brew install`s that formula, and requires the installed version to be
#      the tag's, so the two runs below cannot be reading some other copy.
#   3. Runs `brew test brightfield` — the formula's own `test do` block — and
#      prints its output and exit code.
#   4. Runs `brightfield --check-type-source` from the installed copy, prints its
#      output and exit code. This repeats what the formula's test asserts on
#      purpose: the formula's test can be edited to assert less, and the log has
#      to carry the installed copy's own answer whichever way `brew test` goes.
#
# It runs both 3 and 4 whether or not the other failed, so one red run names
# everything the installed copy gets wrong.
#
# CREDENTIALS: none. The tap is public and the release assets are public, so a
# job running this script needs no secret.
#
# IT MODIFIES THE MACHINE IT RUNS ON — it installs a formula and detaches the
# tap's checkout — so it refuses to run outside CI unless given --allow-local.
#
# Exit codes:
#   0  the installed copy passed `brew test` and the direct check
#   1  the install failed, installed a different version than the tag, or the
#      installed copy failed `brew test` or the direct check
#   2  the check could not run — bad usage, no brew or git, not on CI, the tap
#      would not tap, or the tap holds no formula for the tag. A hard failure in
#      CI, like exit 1: a check that cannot run must not look like one that
#      passed.
#
# Its own regression test is scripts/check-brew-install-selftest.sh, which runs
# on every pull request via .github/workflows/public-hygiene.yml. This script
# needs a published release and a real Homebrew, so it runs on a macOS runner
# from a tag or a dispatch; the self-test is what keeps it honest in between.
set -uo pipefail

usage() {
	cat >&2 <<'USAGE'
usage: check-brew-install.sh --tag vX.Y.Z [--tap OWNER/NAME] [--allow-local]

Installs the tap's formula for the tag, then runs `brew test brightfield` and
`brightfield --check-type-source` from the installed copy. Exit 0 clean, 1 the
installed copy failed, 2 the check could not run.
USAGE
}

TAG=""
TAP="meridian-online/tap"
ALLOW_LOCAL=0
FORMULA_NAME="brightfield"

while [[ $# -gt 0 ]]; do
	case "$1" in
	--tag)
		if [[ $# -lt 2 ]]; then
			echo "check-brew-install: --tag needs a value" >&2
			exit 2
		fi
		TAG="$2"
		shift 2
		;;
	--tag=*)
		TAG="${1#*=}"
		shift
		;;
	--tap)
		if [[ $# -lt 2 ]]; then
			echo "check-brew-install: --tap needs a value" >&2
			exit 2
		fi
		TAP="$2"
		shift 2
		;;
	--tap=*)
		TAP="${1#*=}"
		shift
		;;
	--allow-local)
		ALLOW_LOCAL=1
		shift
		;;
	-h | --help)
		usage
		exit 2
		;;
	*)
		echo "check-brew-install: unknown argument: $1" >&2
		usage
		exit 2
		;;
	esac
done

# A tag is what release.yml's trigger matches: `v` and a digit. A value the shell
# would split or glob is refused here rather than reaching a git argument.
if [[ ! "$TAG" =~ ^v[0-9][0-9A-Za-z._-]*$ ]]; then
	echo "check-brew-install: --tag must look like v1.2.3, got '$TAG'" >&2
	usage
	exit 2
fi
VERSION="${TAG#v}"

if [[ ! "$TAP" =~ ^[A-Za-z0-9._-]+/[A-Za-z0-9._-]+$ ]]; then
	echo "check-brew-install: --tap must look like OWNER/NAME, got '$TAP'" >&2
	exit 2
fi

if [[ "${CI:-}" != "true" && $ALLOW_LOCAL -ne 1 ]]; then
	echo "check-brew-install: refusing to run outside CI — it installs a formula and moves the tap's checkout" >&2
	echo "    on a machine you use, that changes your Homebrew; pass --allow-local if that is what you want" >&2
	exit 2
fi

for tool in brew git; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "check-brew-install: $tool is not on PATH" >&2
		exit 2
	}
done

# Quiet the parts of Homebrew that would change what is being tested or bury the
# log: an auto-update would move the tap off the commit chosen below, and a
# cleanup would remove the Cellar copy's older siblings mid-run.
export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_NO_ANALYTICS=1
export HOMEBREW_NO_ENV_HINTS=1

echo "check-brew-install: $FORMULA_NAME $TAG from $TAP on $(uname -sm)"
brew --version

# ---------------------------------------------------------------------------
# 1. The tap, at the commit that wrote this tag's formula.
# ---------------------------------------------------------------------------
brew tap "$TAP" || {
	echo "check-brew-install: could not tap $TAP" >&2
	exit 2
}

TAPDIR="$(brew --repository "$TAP")" || {
	echo "check-brew-install: brew could not locate the checkout of $TAP" >&2
	exit 2
}
if [[ ! -d "$TAPDIR/.git" ]]; then
	echo "check-brew-install: $TAPDIR is not a git checkout" >&2
	exit 2
fi

# A shallow tap clone would hide the commit an older tag's formula came from.
if [[ "$(git -C "$TAPDIR" rev-parse --is-shallow-repository 2>/dev/null)" == "true" ]]; then
	git -C "$TAPDIR" fetch --quiet --unshallow || {
		echo "check-brew-install: could not deepen the shallow checkout of $TAP" >&2
		exit 2
	}
fi

history="$(git -C "$TAPDIR" log --format='%H %s' -- "Formula/$FORMULA_NAME.rb")" || {
	echo "check-brew-install: could not read the history of Formula/$FORMULA_NAME.rb in $TAP" >&2
	exit 2
}

# The subject is compared whole. A prefix match would take `v0.1.50` for `v0.1.5`.
wanted="Update $FORMULA_NAME to $TAG"
commit=""
while IFS= read -r line; do
	if [[ "${line#* }" == "$wanted" ]]; then
		commit="${line%% *}"
		break
	fi
done <<<"$history"

if [[ -z "$commit" ]]; then
	echo "check-brew-install: $TAP holds no formula written for $TAG (no commit titled '$wanted')" >&2
	echo "    the update-homebrew job in release.yml has not pushed one, or this tag was never released" >&2
	exit 2
fi

git -C "$TAPDIR" checkout --quiet --detach "$commit" || {
	echo "check-brew-install: could not check out $commit in $TAPDIR" >&2
	exit 2
}
echo "check-brew-install: formula from $TAP commit $commit"

# The commit's title says the tag; the file has to say it too, or what gets
# installed is a different release from the one being checked.
if ! grep -Fq "/download/${TAG}/${FORMULA_NAME}-${TAG}-" "$TAPDIR/Formula/$FORMULA_NAME.rb"; then
	echo "check-brew-install: Formula/$FORMULA_NAME.rb at $commit does not address $TAG" >&2
	exit 2
fi

# ---------------------------------------------------------------------------
# 2. The install.
# ---------------------------------------------------------------------------
echo
echo "--- brew install $TAP/$FORMULA_NAME"
brew install "$TAP/$FORMULA_NAME" 2>&1
install_rc=$?
echo "brew install $TAP/$FORMULA_NAME exit=$install_rc"
if [[ $install_rc -ne 0 ]]; then
	echo "FAIL  the formula for $TAG did not install" >&2
	exit 1
fi

installed="$(brew list --versions "$FORMULA_NAME")"
echo "installed: $installed"
if [[ "$installed" != "$FORMULA_NAME $VERSION" ]]; then
	echo "FAIL  the install put '$installed' on the machine, not '$FORMULA_NAME $VERSION'" >&2
	echo "    a test of that copy would say nothing about $TAG" >&2
	exit 1
fi

# ---------------------------------------------------------------------------
# 3 and 4. The formula's own test, then the installed copy's own answer.
# ---------------------------------------------------------------------------
echo
echo "--- brew test $FORMULA_NAME"
brew test "$FORMULA_NAME" 2>&1
test_rc=$?
echo "brew test $FORMULA_NAME exit=$test_rc"

BIN="$(brew --prefix)/bin/$FORMULA_NAME"
echo
echo "--- $FORMULA_NAME --check-type-source, from the installed copy ($BIN)"
if [[ -x "$BIN" ]]; then
	"$BIN" --check-type-source 2>&1
	check_rc=$?
else
	echo "no executable at $BIN"
	check_rc=127
fi
echo "$FORMULA_NAME --check-type-source exit=$check_rc"

echo
failures=0
if [[ $test_rc -ne 0 ]]; then
	echo "FAIL  \`brew test $FORMULA_NAME\` exited $test_rc on the copy Homebrew installed for $TAG" >&2
	failures=$((failures + 1))
fi
if [[ $check_rc -ne 0 ]]; then
	echo "FAIL  \`$FORMULA_NAME --check-type-source\` exited $check_rc from the copy Homebrew installed for $TAG" >&2
	failures=$((failures + 1))
fi
if [[ $failures -ne 0 ]]; then
	echo "check-brew-install: the installed copy of $TAG fails — a person who runs \`brew install\` meets this" >&2
	exit 1
fi

echo "check-brew-install: the installed copy of $TAG passed \`brew test\` and \`--check-type-source\`"
exit 0
