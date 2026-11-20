#!/usr/bin/env bash
# Install a disk image this branch built through Homebrew's cask, four ways, and
# read what each install leaves on the machine.
#
#   scripts/check-brew-install-branch.sh DIST TAG [--allow-local] [--appdir DIR]
#
# DIST holds `brightfield-TAG-aarch64-apple-darwin.dmg`, as scripts/package.sh
# writes it. The cask for each install is written by
# scripts/write-brightfield-cask.sh — the generator release.yml pushes to the
# tap — with its url pointing at a copy of the image on this machine, committed
# to a local git tap under the title release.yml gives the tap's commit, and
# installed with `brew install --cask` (into --appdir when given, else
# /Applications). Nothing is published and no tag is pushed.
#
# WHY IT EXISTS. release.yml's `brew-install` job reads the copy Homebrew
# installs, but only after a tag is pushed and the release is out. A packaging
# or cask change that breaks the installed copy would be published before
# anything read it. This reads it on the pull request that makes the change
# (.github/workflows/brew-install-branch.yml).
#
# WHY IT DOES NOT CALL scripts/check-brew-install.sh. That check requires
# Gatekeeper to report the installed app as notarised, and a branch build
# without the release's signing secrets is signed ad hoc. Here the spctl
# verdict is printed for every install and required of none. The installed
# app's quarantine attribute is then cleared before its command runs, and the
# log says so: launching a quarantined app raises a Gatekeeper dialog, even a
# notarised one on its first launch, and on an unattended runner nobody
# answers it. An app whose signature does not verify is never launched, for
# the same reason. That script's other
# checks — `--check-type-source`, `--check-engine` and the link — are made
# here, with the expectation each install below sets.
#
# THE FOUR INSTALLS, each required to end the way it names:
#
#   packaged        the cask as generated, the image as built. The linked
#                   command passes `--check-type-source` and `--check-engine`,
#                   it links to the cask's exec shim, and the installed
#                   FineType library is byte-identical to the one in the image.
#   symlinked       the same image, the cask's `binary` stanza swapped for the
#                   app's own executable. `--check-type-source` exits 2: the
#                   binary looks for its type source beside the path it was
#                   started by, and through a link in Homebrew's bin it finds
#                   none. This is the fault the shim prevents, reproduced on
#                   every run, so a green `packaged` install is known to depend
#                   on the shim (scripts/write-brightfield-cask.sh says why).
#   tampered        the cask as generated, an image rebuilt with one byte of
#                   the FineType library's code changed and the app re-signed
#                   ad hoc, so its signature is valid and only the bundle
#                   manifest records the change. `--check-type-source` exits 1
#                   saying the library "is not the file that was packaged": the
#                   manifest check still reads every byte.
#   formula-linked  the cask as generated, installed while Homebrew's bin
#                   holds a link into the old formula's Cellar. The cask's
#                   preflight refuses the install, names
#                   `brew uninstall --formula brightfield`, and leaves the link
#                   as it was. The link is a fixture made only when nothing is
#                   at that path, and removed afterwards whatever happens.
#
# There is no `preserve_rpath` install any more. The formula this cask replaced
# needed one, because Homebrew rewrote the install name of a dylib it poured
# into a Cellar; a cask moves the app whole and relocates nothing inside it.
#
# Exit 0 all four ended as required, 1 one did not, 2 the check could not run.
# Refuses to run outside CI unless given --allow-local: it installs a cask,
# taps and untaps a tap and, for one install, links a file into Homebrew's bin.
# It also refuses when a brightfield formula or cask is already installed or
# anything is at Homebrew's bin/brightfield, so it never removes what it did
# not make.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
USAGE="usage: scripts/check-brew-install-branch.sh DIST TAG [--allow-local] [--appdir DIR]"
DIST=""
TAG=""
ALLOW_LOCAL=0
APPDIR=""
while [[ $# -gt 0 ]]; do
	case "$1" in
	--allow-local) ALLOW_LOCAL=1; shift ;;
	--appdir)
		[[ $# -ge 2 ]] || { echo "check-brew-install-branch: --appdir needs a value" >&2; exit 2; }
		APPDIR="$2"; shift 2 ;;
	--appdir=*) APPDIR="${1#*=}"; shift ;;
	-*) echo "check-brew-install-branch: unknown option $1" >&2; exit 2 ;;
	*)
		if [[ -z "$DIST" ]]; then DIST="$1"
		elif [[ -z "$TAG" ]]; then TAG="$1"
		else echo "check-brew-install-branch: unexpected argument $1" >&2; exit 2
		fi
		shift ;;
	esac
done
[[ -n "$DIST" && -n "$TAG" ]] || { echo "$USAGE" >&2; exit 2; }
if [[ ! "$TAG" =~ ^v[0-9][0-9A-Za-z._-]*$ ]]; then
	echo "check-brew-install-branch: TAG must look like v1.2.3, got '$TAG'" >&2
	exit 2
fi
VERSION="${TAG#v}"
if [[ "${CI:-}" != "true" && $ALLOW_LOCAL -ne 1 ]]; then
	echo "check-brew-install-branch: refusing to run outside CI — it installs a cask, taps a local tap" >&2
	echo "    and links a fixture into Homebrew's bin; pass --allow-local if that is what you want" >&2
	exit 2
fi
for tool in brew git hdiutil ditto otool codesign spctl xattr shasum cmp dd od; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "check-brew-install-branch: $tool is not on PATH" >&2
		exit 2
	}
done

NAME="brightfield-${TAG}-aarch64-apple-darwin"
IMAGE="$(cd "$DIST" 2>/dev/null && pwd)/${NAME}.dmg"
[[ -f "$IMAGE" ]] || {
	echo "check-brew-install-branch: no disk image at ${DIST}/${NAME}.dmg" >&2
	exit 2
}

# brew, spctl and the shim may run from another directory than this script was
# started in, so a relative or ~-prefixed --appdir is made absolute here. It is
# then normalised the way Homebrew writes it, so the paths compared below match
# the ones it links: `.`, `..` and a trailing slash resolved, symlinks kept.
# Homebrew 7 expands the appdir without resolving links (`./x/` becomes
# `$PWD/x`, and `/tmp/x` stays `/tmp/x`, not `/private/tmp/x`), which is what
# bash's logical `pwd` gives.
if [[ -n "$APPDIR" ]]; then
	case "$APPDIR" in
	"~") APPDIR="$HOME" ;;
	"~/"*) APPDIR="$HOME/${APPDIR#\~/}" ;;
	esac
	[[ "$APPDIR" == /* ]] || APPDIR="$PWD/$APPDIR"
	mkdir -p "$APPDIR" || exit 2
	APPDIR="$(cd "$APPDIR" && pwd -L)" || exit 2
	APPDIR_ARGS=(--appdir "$APPDIR")
else
	APPDIR="/Applications"
	APPDIR_ARGS=()
fi
APP="$APPDIR/Brightfield.app"
LIB_REL="Contents/Resources/finetype/finetype.dylib"

export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_NO_ANALYTICS=1
export HOMEBREW_NO_ENV_HINTS=1

PREFIX="$(brew --prefix)" || exit 2
BIN="$PREFIX/bin/brightfield"
FIXTURE_TARGET="$PREFIX/Cellar/brightfield/0.0.0-fixture/bin/brightfield"
TAP="brightfield-check/branch"
CASK="$TAP/brightfield"

if brew list --cask brightfield >/dev/null 2>&1 || brew list --formula brightfield >/dev/null 2>&1; then
	echo "check-brew-install-branch: brightfield is already installed; uninstall it first, since this check uninstalls what it installs" >&2
	exit 2
fi
if [[ -e "$BIN" || -L "$BIN" ]]; then
	echo "check-brew-install-branch: $BIN already exists; this check links its own fixture there and will not replace another" >&2
	exit 2
fi
if [[ -e "$APP" ]]; then
	echo "check-brew-install-branch: $APP already exists; pass --appdir to install somewhere else" >&2
	exit 2
fi

WORK="$(mktemp -d)" || exit 2
MOUNTS=()
fixture_made=0
cleanup() {
	brew uninstall --cask --force "$CASK" >/dev/null 2>&1 || true
	if [[ $fixture_made -eq 1 && "$(readlink "$BIN" 2>/dev/null)" == "$FIXTURE_TARGET" ]]; then
		rm -f "$BIN"
	fi
	brew untap "$TAP" >/dev/null 2>&1 || true
	for m in "${MOUNTS[@]+"${MOUNTS[@]}"}"; do hdiutil detach -quiet "$m" >/dev/null 2>&1 || true; done
	rm -rf "$WORK"
}
trap cleanup EXIT

# attach IMAGE MOUNTPOINT [-readonly] — mount without showing it in Finder.
attach() {
	mkdir -p "$2" && hdiutil attach -quiet -nobrowse ${3:+"$3"} -mountpoint "$2" "$1" >/dev/null && MOUNTS+=("$2")
}
detach() { hdiutil detach -quiet "$1" >/dev/null; }

# ── the two images ──────────────────────────────────────────────────────────
# Each under `…/TAG/`, the path write-brightfield-cask.sh requires its base url
# to end in.
GOOD_BASE="$WORK/packaged/$TAG"
BAD_BASE="$WORK/tampered/$TAG"
mkdir -p "$GOOD_BASE" "$BAD_BASE"
cp "$IMAGE" "$GOOD_BASE/${NAME}.dmg" || exit 2

# The image's contents, copied out to a folder the tampered image is built
# from. codesign cannot re-sign the app on a mounted writable image ("internal
# error in Code Signing subsystem"), so the change is made in the copy.
STAGE="$WORK/stage"
PACKAGED_LIB="$WORK/packaged-finetype.dylib"
attach "$GOOD_BASE/${NAME}.dmg" "$WORK/mnt-packaged" -readonly || {
	echo "check-brew-install-branch: could not mount $IMAGE" >&2
	exit 2
}
ditto "$WORK/mnt-packaged" "$STAGE" || exit 2
detach "$WORK/mnt-packaged" || exit 2
cp "$STAGE/Brightfield.app/$LIB_REL" "$PACKAGED_LIB" || {
	echo "check-brew-install-branch: the image's app carries no $LIB_REL" >&2
	exit 2
}

# One byte of the library's machine code inverted: the offset of
# __TEXT,__text plus a page, well inside the code.
LIB="$STAGE/Brightfield.app/$LIB_REL"
text_off="$(otool -l "$LIB" | awk '
	$1 == "sectname" && $2 == "__text" { inside = 1; next }
	inside && $1 == "offset" { print $2; exit }')"
[[ "$text_off" =~ ^[0-9]+$ ]] || {
	echo "check-brew-install-branch: could not find the library's __text section" >&2
	exit 2
}
flip_at=$((text_off + 4096))
byte="$(od -An -tu1 -j "$flip_at" -N1 "$LIB" | tr -d ' ')"
printf '%b' "\\0$(printf '%03o' $((255 - byte)))" |
	dd of="$LIB" bs=1 seek="$flip_at" conv=notrunc 2>/dev/null
if cmp -s "$PACKAGED_LIB" "$LIB"; then
	echo "check-brew-install-branch: the tampered library is identical to the packaged one" >&2
	exit 2
fi

# Then re-signed ad hoc, the library first and then the app, so the app is as
# valid as any ad-hoc branch build and only the bundle manifest records the
# change. Left with its Developer ID seal broken, the app raises Gatekeeper's
# "damaged" dialog when launched even with its quarantine cleared, because
# macOS also assesses a file Homebrew wrote by its com.apple.provenance
# attribute, which cannot be removed; an unattended check must not raise a
# dialog.
codesign --force --sign - "$LIB" 2>/dev/null &&
	codesign --force --sign - "$STAGE/Brightfield.app" 2>/dev/null &&
	codesign --verify --deep --strict "$STAGE/Brightfield.app" || {
	echo "check-brew-install-branch: could not re-sign the tampered app ad hoc" >&2
	exit 2
}
if [[ "$(od -An -tu1 -j "$flip_at" -N1 "$LIB" | tr -d ' ')" != "$((255 - byte))" ]]; then
	echo "check-brew-install-branch: re-signing the tampered library undid its changed byte" >&2
	exit 2
fi
# An explicit size, as scripts/package.sh gives it: with -srcfolder alone,
# hdiutil fails intermittently on this payload.
stage_kb="$(du -sk "$STAGE" | awk '{print $1}')"
hdiutil create -quiet -volname Brightfield -srcfolder "$STAGE" -size "$((stage_kb + 65536))k" \
	-format UDZO -ov "$BAD_BASE/${NAME}.dmg" >/dev/null || {
	echo "check-brew-install-branch: could not build the tampered image" >&2
	exit 2
}
rm -rf "$STAGE"
echo "check-brew-install-branch: tampered image — finetype.dylib byte ${flip_at} (__text at ${text_off}) ${byte} -> $((255 - byte))"

sha_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
GOOD_SHA="$(sha_of "$GOOD_BASE/${NAME}.dmg")"
BAD_SHA="$(sha_of "$BAD_BASE/${NAME}.dmg")"

# ── the local tap ───────────────────────────────────────────────────────────
# A git repository tapped by path. Homebrew clones it, so each cask below is
# committed in the clone Homebrew reads.
brew untap "$TAP" >/dev/null 2>&1 || true
SEED="$WORK/tap-seed"
gitq() { git -C "$1" -c user.name=brew-install-branch -c user.email=brew-install-branch@example.invalid -c commit.gpgsign=false "${@:2}"; }
mkdir -p "$SEED" && gitq "$SEED" init -q && gitq "$SEED" commit -q --allow-empty -m "Seed the branch check's tap" || exit 2
brew tap "$TAP" "$SEED" >/dev/null 2>&1 || {
	echo "check-brew-install-branch: could not tap $TAP from $SEED" >&2
	exit 2
}
TAPDIR="$(brew --repository "$TAP")" || exit 2

# publish BASE SHA SHAPE — write the cask and commit it under the title
# release.yml gives the tap's commit. SHAPE `shim` is the cask as generated;
# `symlinked` links the app's own executable in place of the shim. Only the
# aarch64 image exists here, so its digest stands in for the Intel one.
publish() {
	local base="$1" sha="$2" shape="$3" cask="$TAPDIR/Casks/brightfield.rb"
	mkdir -p "$TAPDIR/Casks"
	"$HERE/write-brightfield-cask.sh" "$TAG" "file://$base" "$sha" "$sha" >"$cask" || return 2
	"$HERE/check-cask-layout.sh" "$cask" >/dev/null || return 2
	if [[ "$shape" == symlinked ]]; then
		sed 's|^  binary "brightfield.wrapper.sh", target: "brightfield"$|  binary "#{appdir}/Brightfield.app/Contents/MacOS/brightfield", target: "brightfield"|' \
			"$cask" >"$cask.new" && mv "$cask.new" "$cask"
		if grep -q 'binary "brightfield.wrapper.sh"' "$cask"; then
			echo "check-brew-install-branch: could not swap the shim out of the cask" >&2
			return 2
		fi
	fi
	gitq "$TAPDIR" add Casks && gitq "$TAPDIR" commit -q -m "Update brightfield to $TAG" || return 2
}

failures=0
fail() { echo "FAIL  $*" >&2; failures=$((failures + 1)); }
pass() { echo "ok    $*"; }

# brew_install NAME — install the cask the tap holds now. Sets install_rc and
# leaves brew's output in $WORK/NAME.log.
brew_install() {
	echo "--- brew install --cask ${APPDIR_ARGS[*]+"${APPDIR_ARGS[*]}"} $CASK"
	brew install --cask "${APPDIR_ARGS[@]+"${APPDIR_ARGS[@]}"}" "$CASK" >"$WORK/$1.log" 2>&1
	install_rc=$?
	sed 's/^/  | /' "$WORK/$1.log"
	echo "brew install exit=$install_rc"
}

# gatekeeper NAME — print spctl's verdict on the installed app, then clear its
# quarantine before anything in it runs. Launching a quarantined app raises a
# Gatekeeper dialog — "damaged" for one Gatekeeper rejects, and a first-launch
# confirmation even for a notarised one — and the launch waits on it, with
# nobody there to answer. spctl has already given its verdict, so clearing the
# attribute changes nothing this check reads. Returns non-zero, so the caller
# runs nothing, when the app's signature does not verify or its quarantine
# could not be cleared; either would raise that dialog.
gatekeeper() {
	local verdict rc sig
	verdict="$(spctl -a -vv -t exec "$APP" 2>&1)"
	rc=$?
	echo "--- spctl -a -vv -t exec $APP (exit $rc)"
	printf '%s\n' "$verdict" | sed 's/^/  | /'
	if [[ $rc -eq 0 && "$verdict" == *"source=Notarized Developer ID"* ]]; then
		echo "spctl reports the app notarised"
	else
		echo "spctl does not report the app notarised"
	fi
	xattr -dr com.apple.quarantine "$APP"
	if xattr -r "$APP" 2>/dev/null | grep -q 'com\.apple\.quarantine$'; then
		fail "$1: $APP still carries com.apple.quarantine, so its command was not run"
		return 1
	fi
	echo "cleared com.apple.quarantine on $APP, so launching it raises no Gatekeeper dialog"
	if ! sig="$(codesign --verify --deep --strict "$APP" 2>&1)"; then
		printf '%s\n' "$sig" | sed 's/^/  | /'
		fail "$1: the installed app's signature does not verify, so its command was not run (launching it would raise Gatekeeper's dialog)"
		return 1
	fi
}

# run_bin FLAG — run the linked command and print its output; sets rc and out.
run_bin() {
	echo "--- brightfield $1, from the linked command ($BIN)"
	if [[ -x "$BIN" ]]; then
		out="$("$BIN" "$1" 2>&1)"
		rc=$?
	else
		out="no executable at $BIN"
		rc=127
	fi
	printf '%s\n' "$out" | sed 's/^/  | /'
	echo "brightfield $1 exit=$rc"
}

# start NAME — a heading, and nothing left over from the install before.
start() {
	echo
	echo "=== $1"
	brew uninstall --cask --force "$CASK" >/dev/null 2>&1 || true
}

SHIM="$PREFIX/Caskroom/brightfield/$VERSION/brightfield.wrapper.sh"
APP_BIN="$APP/Contents/MacOS/brightfield"

# ── packaged ────────────────────────────────────────────────────────────────
publish "$GOOD_BASE" "$GOOD_SHA" shim || exit 2
LEG=packaged
start packaged
brew_install packaged
if [[ $install_rc -ne 0 ]]; then
	fail "packaged: the cask did not install (exit $install_rc)"
elif gatekeeper "$LEG"; then
	run_bin --check-type-source
	if [[ $rc -eq 0 ]]; then pass "packaged: --check-type-source exited 0"
	else fail "packaged: --check-type-source exited $rc, not 0"; fi
	run_bin --check-engine
	if [[ $rc -eq 0 ]]; then pass "packaged: --check-engine exited 0"
	else fail "packaged: --check-engine exited $rc, not 0"; fi
	link="$(readlink "$BIN" 2>/dev/null)"
	if [[ "$link" == "$SHIM" ]]; then pass "packaged: $BIN links to the cask's shim"
	else fail "packaged: $BIN links to '${link:-nothing}', not $SHIM"; fi
	if cmp -s "$PACKAGED_LIB" "$APP/$LIB_REL"; then
		pass "packaged: the installed finetype.dylib is byte-identical to the image's"
	else
		fail "packaged: the installed finetype.dylib differs from the image's"
	fi
fi

# ── symlinked ───────────────────────────────────────────────────────────────
publish "$GOOD_BASE" "$GOOD_SHA" symlinked || exit 2
LEG=symlinked
start symlinked
brew_install symlinked
if [[ $install_rc -ne 0 ]]; then
	fail "symlinked: the cask did not install (exit $install_rc)"
elif gatekeeper "$LEG"; then
	link="$(readlink "$BIN" 2>/dev/null)"
	if [[ "$link" == "$APP_BIN" ]]; then pass "symlinked: $BIN links to the app's own executable"
	else fail "symlinked: $BIN links to '${link:-nothing}', not $APP_BIN"; fi
	run_bin --check-type-source
	if [[ $rc -eq 2 ]]; then pass "symlinked: --check-type-source exited 2, finding no type source through the link"
	else fail "symlinked: --check-type-source exited $rc, not 2"; fi
fi

# ── tampered ────────────────────────────────────────────────────────────────
publish "$BAD_BASE" "$BAD_SHA" shim || exit 2
LEG=tampered
start tampered
brew_install tampered
if [[ $install_rc -ne 0 ]]; then
	fail "tampered: the cask did not install (exit $install_rc)"
elif gatekeeper "$LEG"; then
	run_bin --check-type-source
	needle="finetype.dylib is not the file that was packaged"
	if [[ $rc -eq 1 && "$out" == *"$needle"* ]]; then
		pass "tampered: --check-type-source exited 1 and printed: $needle"
	else
		fail "tampered: --check-type-source exited $rc, not 1 printing: $needle"
	fi
fi

# ── formula-linked ──────────────────────────────────────────────────────────
publish "$GOOD_BASE" "$GOOD_SHA" shim || exit 2
start formula-linked
if [[ -e "$BIN" || -L "$BIN" ]]; then
	fail "formula-linked: $BIN is still there after the last uninstall, so no fixture was made"
else
	ln -s "$FIXTURE_TARGET" "$BIN" && fixture_made=1
	echo "fixture: $BIN -> $(readlink "$BIN")"
	brew_install formula-linked
	if [[ $install_rc -ne 0 ]]; then pass "formula-linked: the install was refused (exit $install_rc)"
	else fail "formula-linked: the install succeeded over the formula's link"; fi
	if grep -qF "brew uninstall --formula brightfield" "$WORK/formula-linked.log"; then
		pass "formula-linked: the refusal names brew uninstall --formula brightfield"
	else
		fail "formula-linked: the output does not name brew uninstall --formula brightfield"
	fi
	if [[ -L "$BIN" && "$(readlink "$BIN")" == "$FIXTURE_TARGET" ]]; then
		pass "formula-linked: $BIN is unchanged"
	else
		fail "formula-linked: $BIN changed to '$(readlink "$BIN" 2>/dev/null)'"
	fi
	# A refused install leaves its staging directory behind and names it. The
	# path comes from brew's output, so it is removed only when it is plainly
	# that directory: under /tmp, named for this cask, and with no `..` in it.
	retained="$(grep -A1 '^Warning: Temporary files retained at:' "$WORK/formula-linked.log" | sed -n 2p)"
	if [[ "$retained" != *..* ]]; then
		case "$retained" in
		/private/tmp/cask-brightfield-* | /tmp/cask-brightfield-*) rm -rf "$retained" ;;
		esac
	fi
	brew uninstall --cask --force "$CASK" >/dev/null 2>&1 || true
	if [[ "$(readlink "$BIN" 2>/dev/null)" == "$FIXTURE_TARGET" ]]; then
		rm -f "$BIN"
	fi
	fixture_made=0
fi

echo
if [[ $failures -ne 0 ]]; then
	echo "check-brew-install-branch: ${failures} check(s) did not end as required" >&2
	exit 1
fi
echo "check-brew-install-branch: Homebrew installs this image with its type source intact through the shim, and the symlinked, tampered and formula-linked installs end as required"
exit 0
