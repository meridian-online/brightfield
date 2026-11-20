#!/usr/bin/env bash
# Install the Homebrew cask the tap holds for a published tag, then check the
# app Homebrew installed and the command it linked.
#
#   scripts/check-brew-install.sh --tag vX.Y.Z [--tap OWNER/NAME] [--allow-local] [--appdir DIR]
#
# WHY THIS EXISTS. The release's other checks read the package BEFORE Homebrew
# has touched it: package.sh builds the disk image, the type-source check reads
# the binary inside it, and scripts/check-cask-asset.sh hashes the asset the
# cask addresses. `brew install --cask` then moves the app into the application
# directory and links a command into Homebrew's bin, and those are what a person
# runs. No check read them, so a fault that exists after Homebrew's install
# reached a person's machine before it reached anyone here, and the release run
# that published it was green.
#
# WHAT IT DOES, in order:
#
#   1. Taps the tap and moves its checkout to the commit that wrote the cask
#      for --tag (the commit titled "Update brightfield to <tag>", which is what
#      the update-homebrew job in release.yml commits, and which also deletes
#      the old formula). In a release run that is the commit the previous job
#      just pushed. Dispatched by hand for an older tag, it is the cask the tap
#      held for that tag, not whatever the tap holds now.
#   2. `brew install --cask`s it (into --appdir when given, else Homebrew's
#      default, /Applications), and requires the installed version to be the
#      tag's, so the checks below cannot be reading some other copy.
#   3. Runs four checks, each reported whether or not the others failed:
#        - `spctl -a -vv -t exec` on the installed app must say
#          `source=Notarized Developer ID`: Gatekeeper's own verdict, which is
#          what a person's Mac applies on first launch. A cask that mangled the
#          app, or a release that shipped it ad hoc, fails here.
#        - `brightfield --check-type-source` through the linked command exits 0.
#        - `brightfield --check-engine` through the linked command exits 0.
#        - the linked command resolves to the cask's exec shim, not to the
#          app's own binary. The binary finds its bundled type source beside
#          its own path, and through a symlink it reports Homebrew's bin as
#          that path and finds none (see scripts/write-brightfield-cask.sh).
#          The type-source check above would catch that too; the link is read
#          on its own so the log names the cause.
#
# There is no `brew test` step. Casks have no `test do` block, so the checks
# above are the whole of what is read from the installed copy.
#
# CREDENTIALS: none. The tap is public and the release assets are public, so a
# job running this script needs no secret.
#
# IT MODIFIES THE MACHINE IT RUNS ON — it installs a cask, which moves an app
# into the application directory, and it detaches the tap's checkout — so it
# refuses to run outside CI unless given --allow-local.
#
# Exit codes:
#   0  the installed app passed all four checks
#   1  the install failed, installed a different version than the tag, or the
#      installed copy failed any check
#   2  the check could not run — bad usage, no brew or git, not on CI, the tap
#      would not tap, or the tap holds no cask for the tag. A hard failure in
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
usage: check-brew-install.sh --tag vX.Y.Z [--tap OWNER/NAME] [--allow-local] [--appdir DIR]

Installs the tap's cask for the tag, then checks the installed app with spctl
and runs `brightfield --check-type-source` and `brightfield --check-engine`
through the linked command. Exit 0 clean, 1 the installed copy failed, 2 the
check could not run.
USAGE
}

TAG=""
TAP="meridian-online/tap"
ALLOW_LOCAL=0
CASK_NAME="brightfield"
APPDIR=""

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
	--appdir)
		if [[ $# -lt 2 ]]; then
			echo "check-brew-install: --appdir needs a value" >&2
			exit 2
		fi
		APPDIR="$2"
		shift 2
		;;
	--appdir=*)
		APPDIR="${1#*=}"
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
	echo "check-brew-install: refusing to run outside CI — it installs a cask and moves the tap's checkout" >&2
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

echo "check-brew-install: $CASK_NAME $TAG from $TAP on $(uname -sm)"
brew --version

# ---------------------------------------------------------------------------
# 1. The tap, at the commit that wrote this tag's cask.
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

# A shallow tap clone would hide the commit an older tag's cask came from.
if [[ "$(git -C "$TAPDIR" rev-parse --is-shallow-repository 2>/dev/null)" == "true" ]]; then
	git -C "$TAPDIR" fetch --quiet --unshallow || {
		echo "check-brew-install: could not deepen the shallow checkout of $TAP" >&2
		exit 2
	}
fi

history="$(git -C "$TAPDIR" log --format='%H %s' -- "Casks/$CASK_NAME.rb")" || {
	echo "check-brew-install: could not read the history of Casks/$CASK_NAME.rb in $TAP" >&2
	exit 2
}

# The subject is compared whole. A prefix match would take `v0.1.50` for `v0.1.5`.
wanted="Update $CASK_NAME to $TAG"
commit=""
while IFS= read -r line; do
	if [[ "${line#* }" == "$wanted" ]]; then
		commit="${line%% *}"
		break
	fi
done <<<"$history"

if [[ -z "$commit" ]]; then
	echo "check-brew-install: $TAP holds no cask written for $TAG (no commit titled '$wanted')" >&2
	echo "    the update-homebrew job in release.yml has not pushed one, or this tag was never released" >&2
	exit 2
fi

git -C "$TAPDIR" checkout --quiet --detach "$commit" || {
	echo "check-brew-install: could not check out $commit in $TAPDIR" >&2
	exit 2
}
echo "check-brew-install: cask from $TAP commit $commit"

# The commit's title says the tag; the file has to say it too, or what gets
# installed is a different release from the one being checked. The cask's urls
# are written from its version, so the version line is where the tag shows.
if ! grep -Fxq "  version \"${VERSION}\"" "$TAPDIR/Casks/$CASK_NAME.rb"; then
	echo "check-brew-install: Casks/$CASK_NAME.rb at $commit does not address $TAG" >&2
	exit 2
fi

# ---------------------------------------------------------------------------
# 2. The install.
# ---------------------------------------------------------------------------
install_args=(install --cask)
if [[ -n "$APPDIR" ]]; then
	install_args+=(--appdir "$APPDIR")
else
	APPDIR="/Applications"
fi
APP="$APPDIR/Brightfield.app"

echo
echo "--- brew ${install_args[*]} $TAP/$CASK_NAME"
brew "${install_args[@]}" "$TAP/$CASK_NAME" 2>&1
install_rc=$?
echo "brew install exit=$install_rc"
if [[ $install_rc -ne 0 ]]; then
	echo "FAIL  the cask for $TAG did not install" >&2
	exit 1
fi

installed="$(brew list --cask --versions "$CASK_NAME")"
echo "installed: $installed"
if [[ "$installed" != "$CASK_NAME $VERSION" ]]; then
	echo "FAIL  the install put '$installed' on the machine, not '$CASK_NAME $VERSION'" >&2
	echo "    a check of that copy would say nothing about $TAG" >&2
	exit 1
fi

# ---------------------------------------------------------------------------
# 3. The installed app and the command Homebrew linked, each read on its own.
# ---------------------------------------------------------------------------
echo
echo "--- spctl -a -vv -t exec $APP"
# spctl writes its verdict to stderr.
spctl_out="$(spctl -a -vv -t exec "$APP" 2>&1)"
spctl_rc=$?
printf '%s\n' "$spctl_out"
echo "spctl exit=$spctl_rc"

BIN="$(brew --prefix)/bin/$CASK_NAME"

run_installed() {
	# `run_installed <flag>` — prints the linked command's output and sets rc.
	echo
	echo "--- $CASK_NAME $1, from the linked command ($BIN)"
	if [[ -x "$BIN" ]]; then
		"$BIN" "$1" 2>&1
		rc=$?
	else
		echo "no executable at $BIN"
		rc=127
	fi
	echo "$CASK_NAME $1 exit=$rc"
}
run_installed --check-type-source
check_rc=$rc
run_installed --check-engine
engine_rc=$rc

echo
echo "--- the link at $BIN"
link_target="$(readlink "$BIN" 2>/dev/null)"
echo "readlink: ${link_target:-<not a symlink>}"

echo
failures=0
if [[ $spctl_rc -ne 0 || "$spctl_out" != *"source=Notarized Developer ID"* ]]; then
	echo "FAIL  Gatekeeper does not accept $APP as Notarized Developer ID (spctl exit $spctl_rc)" >&2
	failures=$((failures + 1))
fi
if [[ $check_rc -ne 0 ]]; then
	echo "FAIL  \`$CASK_NAME --check-type-source\` exited $check_rc from the copy Homebrew installed for $TAG" >&2
	failures=$((failures + 1))
fi
if [[ $engine_rc -ne 0 ]]; then
	echo "FAIL  \`$CASK_NAME --check-engine\` exited $engine_rc from the copy Homebrew installed for $TAG" >&2
	failures=$((failures + 1))
fi
if [[ "$(basename "${link_target:-}")" != "brightfield.wrapper.sh" || "$link_target" == *"/Contents/MacOS/"* ]]; then
	echo "FAIL  $BIN links to '${link_target:-nothing}', not the cask's exec shim brightfield.wrapper.sh" >&2
	failures=$((failures + 1))
fi
if [[ $failures -ne 0 ]]; then
	echo "check-brew-install: the installed copy of $TAG fails — a person who runs \`brew install --cask\` meets this" >&2
	exit 1
fi

echo "check-brew-install: the installed copy of $TAG is notarised, answers both checks, and is linked through the shim"
exit 0
