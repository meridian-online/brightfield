#!/usr/bin/env bash
# Install a package this branch built through Homebrew, three ways, and read
# what each install leaves in the Cellar.
#
#   scripts/check-brew-install-branch.sh DIST TAG [--allow-local]
#
# DIST holds `brightfield-TAG-aarch64-apple-darwin.tar.gz`, as scripts/package.sh
# writes it. The formula for each install is written by
# scripts/write-brightfield-formula.sh — the generator release.yml pushes to the
# tap — with its url pointing at a file on this machine, committed to a local
# tap under the title release.yml gives the tap's commit, and installed and run
# by scripts/check-brew-install.sh, the same check release.yml runs on the
# published formula. Nothing is published and no tag is pushed.
#
# WHY IT EXISTS. release.yml's `brew-install` job reads the copy Homebrew
# installs, but only after a tag is pushed and the release is out. A packaging
# change that Homebrew's install undoes — as it undid the finetype extension in
# 0.1.5, by rewriting its install name and re-signing it — was published before
# anything read the installed copy. This reads it on the pull request that makes
# the change (.github/workflows/brew-install-branch.yml).
#
# THE THREE INSTALLS, each required to end the way it names:
#
#   packaged   the formula as generated, the package as built. The installed
#              copy passes `brew test` and `--check-type-source` types a column,
#              and the installed extension is byte-identical to the packaged one.
#   renamed    the same package, the formula with its `preserve_rpath` line
#              removed. Homebrew rewrites the extension's install name and
#              re-signs it, and the installed copy refuses it as not the file
#              that was packaged. This is the fault the line prevents, reproduced
#              on every run, so a green `packaged` install is known to depend on
#              that line rather than on Homebrew having stopped renaming dylibs.
#   tampered   the formula as generated, the package with one byte of the
#              extension's code changed after packaging. The installed copy
#              refuses it: the manifest check still reads every byte.
#
# After each install the packaged and the installed extension are compared —
# digest, install name, signature flags and the count of differing bytes — so
# the log says what the install did to the file, not only whether the check
# passed.
#
# Exit 0 all three ended as required, 1 one did not, 2 the check could not run.
# Refuses to run outside CI unless given --allow-local: it installs formulas and
# writes a tap into Homebrew's own directory.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DIST=""
TAG=""
ALLOW_LOCAL=0
for arg in "$@"; do
	case "$arg" in
	--allow-local) ALLOW_LOCAL=1 ;;
	-*) echo "check-brew-install-branch: unknown option $arg" >&2; exit 2 ;;
	*)
		if [[ -z "$DIST" ]]; then DIST="$arg"
		elif [[ -z "$TAG" ]]; then TAG="$arg"
		else echo "check-brew-install-branch: unexpected argument $arg" >&2; exit 2
		fi
		;;
	esac
done
[[ -n "$DIST" && -n "$TAG" ]] || {
	echo "usage: scripts/check-brew-install-branch.sh DIST TAG [--allow-local]" >&2
	exit 2
}
if [[ ! "$TAG" =~ ^v[0-9][0-9A-Za-z._-]*$ ]]; then
	echo "check-brew-install-branch: TAG must look like v1.2.3, got '$TAG'" >&2
	exit 2
fi
if [[ "${CI:-}" != "true" && $ALLOW_LOCAL -ne 1 ]]; then
	echo "check-brew-install-branch: refusing to run outside CI — it uninstalls any installed brightfield," >&2
	echo "    installs formulas and writes a tap into Homebrew's directory; pass --allow-local if that is what you want" >&2
	exit 2
fi
for tool in brew git tar otool codesign shasum cmp; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "check-brew-install-branch: $tool is not on PATH" >&2
		exit 2
	}
done

TARGET="aarch64-apple-darwin"
NAME="brightfield-${TAG}-${TARGET}"
TARBALL="$(cd "$DIST" 2>/dev/null && pwd)/${NAME}.tar.gz"
[[ -f "$TARBALL" ]] || {
	echo "check-brew-install-branch: no package at ${DIST}/${NAME}.tar.gz" >&2
	exit 2
}

export HOMEBREW_NO_AUTO_UPDATE=1
export HOMEBREW_NO_INSTALL_CLEANUP=1
export HOMEBREW_NO_ANALYTICS=1
export HOMEBREW_NO_ENV_HINTS=1

WORK="$(mktemp -d)" || exit 2
TAP="brightfield-check/branch"
TAPDIR="$(brew --repository)/Library/Taps/brightfield-check/homebrew-branch"
cleanup() {
	brew uninstall --formula --force brightfield >/dev/null 2>&1 || true
	rm -rf "$TAPDIR" "$WORK"
}
trap cleanup EXIT

# ── the two packages ────────────────────────────────────────────────────────
# Each under `…/download/TAG/`, the path check-brew-install.sh requires a formula
# for TAG to address.
GOOD_BASE="$WORK/packaged/download/$TAG"
BAD_BASE="$WORK/tampered/download/$TAG"
mkdir -p "$GOOD_BASE" "$BAD_BASE" "$WORK/unpack"
cp "$TARBALL" "$GOOD_BASE/"
PACKAGED_EXT="$WORK/packaged-extension"
tar -xzf "$TARBALL" -C "$WORK/unpack" || {
	echo "check-brew-install-branch: could not unpack $TARBALL" >&2
	exit 2
}
EXT_REL="${NAME}/finetype/finetype.duckdb_extension"
cp "$WORK/unpack/$EXT_REL" "$PACKAGED_EXT" || {
	echo "check-brew-install-branch: the package carries no ${EXT_REL#*/}" >&2
	exit 2
}

# One byte of the extension's machine code, inverted, after the manifest was
# written: the offset of __TEXT,__text plus a page, well inside the code.
text_off="$(otool -l "$PACKAGED_EXT" | awk '
	$1 == "sectname" && $2 == "__text" { inside = 1; next }
	inside && $1 == "offset" { print $2; exit }')"
[[ "$text_off" =~ ^[0-9]+$ ]] || {
	echo "check-brew-install-branch: could not find the extension's __text section" >&2
	exit 2
}
flip_at=$((text_off + 4096))
byte="$(od -An -tu1 -j "$flip_at" -N1 "$PACKAGED_EXT" | tr -d ' ')"
printf '%b' "\\0$(printf '%03o' $((255 - byte)))" |
	dd of="$WORK/unpack/$EXT_REL" bs=1 seek="$flip_at" conv=notrunc 2>/dev/null
if cmp -s "$PACKAGED_EXT" "$WORK/unpack/$EXT_REL"; then
	echo "check-brew-install-branch: the tampered extension is identical to the packaged one" >&2
	exit 2
fi
tar -czf "$BAD_BASE/${NAME}.tar.gz" -C "$WORK/unpack" "$NAME" || exit 2
echo "check-brew-install-branch: tampered package — extension byte ${flip_at} (__text at ${text_off}) ${byte} -> $((255 - byte))"

sha_of() { shasum -a 256 "$1" | cut -d' ' -f1; }
GOOD_SHA="$(sha_of "$GOOD_BASE/${NAME}.tar.gz")"
BAD_SHA="$(sha_of "$BAD_BASE/${NAME}.tar.gz")"

# ── the local tap ───────────────────────────────────────────────────────────
rm -rf "$TAPDIR"
mkdir -p "$TAPDIR/Formula"
gitq() { git -C "$TAPDIR" -c user.name=brew-install-branch -c user.email=brew-install-branch@example.invalid -c commit.gpgsign=false "$@"; }
gitq init -q || exit 2

# publish BASE SHA KEEP_PRESERVE — write the formula, commit it under the title
# check-brew-install.sh looks for.
publish() {
	local base="$1" sha="$2" keep="$3" formula="$TAPDIR/Formula/brightfield.rb"
	"$HERE/write-brightfield-formula.sh" "$TAG" "file://$base" "$sha" "$sha" >"$formula" || return 2
	if [[ "$keep" == keep ]]; then
		"$HERE/check-formula-layout.sh" "$formula" || return 2
	else
		grep -v 'preserve_rpath if' "$formula" >"$formula.new" && mv "$formula.new" "$formula"
		if grep -q '^[[:space:]]*preserve_rpath' "$formula"; then
			echo "check-brew-install-branch: could not remove preserve_rpath from the formula" >&2
			return 2
		fi
	fi
	gitq add Formula && gitq commit -q -m "Update brightfield to $TAG" || return 2
}

# compare — what the install did to the extension, packaged against installed.
compare() {
	local installed
	installed="$(brew --prefix brightfield 2>/dev/null)/libexec/finetype/finetype.duckdb_extension"
	if [[ ! -f "$installed" ]]; then
		echo "  installed extension: none at $installed"
		return
	fi
	for which in packaged installed; do
		local f="$PACKAGED_EXT"
		[[ "$which" == installed ]] && f="$installed"
		printf '  %-9s sha256 %s  install name %s  %s\n' "$which" "$(sha_of "$f")" \
			"$(otool -D "$f" | sed -n 2p)" "$(codesign -dv "$f" 2>&1 | grep -o 'flags=[^ ]*')"
	done
	echo "  bytes that differ: $(cmp -l "$PACKAGED_EXT" "$installed" 2>/dev/null | wc -l | tr -d ' ')"
}

failures=0
# install_leg NAME WANT_EXIT NEEDLE — run the installed-copy check; it must exit
# WANT_EXIT and print NEEDLE.
install_leg() {
	local name="$1" want="$2" needle="$3" log="$WORK/$1.log" rc
	echo
	echo "=== ${name}"
	brew uninstall --formula --force brightfield >/dev/null 2>&1 || true
	"$HERE/check-brew-install.sh" --tag "$TAG" --tap "$TAP" >"$log" 2>&1
	rc=$?
	sed 's/^/  | /' "$log"
	compare
	if [[ $rc -eq 2 ]]; then
		echo "FAIL  ${name}: the installed-copy check could not run (exit 2)" >&2
		failures=$((failures + 1))
	elif [[ $rc -ne $want ]]; then
		echo "FAIL  ${name}: the installed-copy check exited ${rc}, not ${want}" >&2
		failures=$((failures + 1))
	elif ! grep -qF -- "$needle" "$log"; then
		echo "FAIL  ${name}: exited ${rc} without printing: ${needle}" >&2
		failures=$((failures + 1))
	else
		echo "ok    ${name}: exited ${rc} and printed: ${needle}"
	fi
}

publish "$GOOD_BASE" "$GOOD_SHA" keep || exit 2
install_leg packaged 0 "typed as"
installed_ext="$(brew --prefix brightfield 2>/dev/null)/libexec/finetype/finetype.duckdb_extension"
if [[ -f "$installed_ext" ]] && cmp -s "$PACKAGED_EXT" "$installed_ext"; then
	echo "ok    packaged: the installed extension is byte-identical to the packaged one"
else
	echo "FAIL  packaged: the installed extension is not byte-identical to the packaged one" >&2
	failures=$((failures + 1))
fi

publish "$GOOD_BASE" "$GOOD_SHA" drop || exit 2
install_leg renamed 1 "is not the file that was packaged"

publish "$BAD_BASE" "$BAD_SHA" keep || exit 2
install_leg tampered 1 "is not the file that was packaged"

echo
if [[ $failures -ne 0 ]]; then
	echo "check-brew-install-branch: ${failures} install(s) did not end as required" >&2
	exit 1
fi
echo "check-brew-install-branch: Homebrew installs this package byte-identical, and the check refuses the renamed and the tampered extension"
exit 0
