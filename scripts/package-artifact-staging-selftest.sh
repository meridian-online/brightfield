#!/usr/bin/env bash
# Prove a REAL scripts/package.sh run puts the type source inside every artifact
# it produces, at the path that artifact's own reader opens — and the DuckDB CLI
# the Run control drives, at the path the shell's runner looks for it.
#
# WHAT WAS UNPINNED UNTIL THIS FILE. `stage_finetype` is called twice — once
# into the tarball's staging tree and once into `Contents/Resources` of the app
# bundle — and no check on a pull request could see either call. Deleting
# `stage_finetype "$APP/Contents/Resources/finetype"`, or reducing it to a
# `mkdir -p`, left every check in this repository green; the disk image then
# ships an application that reports storage types for every column, and the only
# thing that would have caught it is the read-back on a tag, which is also the
# moment it is least welcome to be wrong. Measured on the commit this file was
# written against, both mutations, both green.
#
# scripts/package-finetype-selftest.sh cannot reach it: every case there stops at
# or just after the bundle check, before the compiler, which is what lets it run
# on a runner with no Rust. That ordering is deliberate and worth keeping. This
# file is the other half — it runs packaging to completion, and pays for it.
#
# HOW IT AVOIDS A REAL BUILD. `cargo` is stubbed with a script that compiles a short
# C program to the path package.sh reads. That is a genuine Mach-O: `otool -L`
# lists libSystem for the linkage audit, `otool -l` carries the LC_BUILD_VERSION
# the bundle's LSMinimumSystemVersion is read out of, and `codesign` signs it.
# It answers `--check-type-source` with a marker and exit 0, so the read-back's
# run leg is genuinely reached and genuinely executes a binary out of the
# artifact rather than being skipped.
#
# WHAT THAT STUB DOES NOT ESTABLISH, said plainly rather than left implied: it
# says nothing about whether the real brightfield loads the real extension. That
# is what the read-back does on a tag with the real binary, and what
# scripts/check-artifact-type-source-selftest.sh drives with fixtures. What this
# file establishes is the STAGING — that the bundle is in the artifact, at the
# path the reader opens, in both artifacts, after a real packaging run.
#
# EACH MUTATION IS DRIVEN, NOT DESCRIBED. The three ways staging has been broken
# are applied to package.sh in a throwaway copy of the checkout and the read-back
# is required to refuse each, naming the path it looked in. A check that passes
# over correct packaging and is never shown failing over broken packaging is a
# check nobody has evidence about.
#
# macOS only, and it must be: the app bundle and the disk image are darwin paths
# in scripts/package.sh, and `hdiutil`, `codesign` and `plutil` are macOS tools.
# test.yml runs it on macos-15. On any other system it exits 0 having run
# nothing, and says so — the ubuntu hygiene runner must not read that as a pass.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"

if [ "$(uname -s)" != "Darwin" ]; then
	echo "package-artifact-staging-selftest: NOT APPLICABLE on $(uname -s)."
	echo "  scripts/package.sh builds the app bundle and the disk image only for"
	echo "  *-apple-darwin, and hdiutil/codesign/plutil are macOS tools. test.yml"
	echo "  runs this file on macos-15; nothing here ran."
	exit 0
fi

for tool in cc otool codesign plutil hdiutil shasum rustc rsync; do
	command -v "$tool" >/dev/null 2>&1 || {
		echo "package-artifact-staging-selftest: ${tool} is missing on a Darwin host" >&2
		exit 1
	}
done

TARGET="$(rustc -vV | sed -n 's/^host: //p')"
[ -n "$TARGET" ] || {
	echo "package-artifact-staging-selftest: rustc -vV printed no 'host:' line" >&2
	exit 1
}
case "$TARGET" in
*-apple-darwin) ;;
*)
	echo "package-artifact-staging-selftest: the host is ${TARGET}, not a darwin target" >&2
	exit 1
	;;
esac
PLATFORM="$("$HERE/duckdb-platform.sh" "$TARGET")"
TAG="$("$HERE/finetype-pin.sh")"
CRATE_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/brightfield-shell/Cargo.toml" | head -1)"
NAME="brightfield-v${CRATE_VERSION}-${TARGET}"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
out="$TMP/out"

# ── the fixture bundle ──────────────────────────────────────────────────────
# The shape scripts/check-bundled-extension.sh accepts, for this platform and
# the pinned tag, so packaging gets past its own refusals and reaches staging.
BUNDLE="$TMP/bundle"
mkdir -p "$BUNDLE/model/model2vec" "$BUNDLE/model/value_model2vec"
"$HERE/fixture-extension.py" "$BUNDLE/finetype.duckdb_extension" "$PLATFORM" v1.2.0 "${TAG#v}" C_STRUCT
printf 'weights' >"$BUNDLE/model/model.safetensors"
printf '{"value_embed_model": "value_model2vec"}' >"$BUNDLE/model/config.json"
printf '{}' >"$BUNDLE/model/label_map.json"
printf 'weights' >"$BUNDLE/model/model2vec/model.safetensors"
printf '{}' >"$BUNDLE/model/model2vec/tokenizer.json"
printf 'weights' >"$BUNDLE/model/value_model2vec/model.safetensors"
printf '{}' >"$BUNDLE/model/value_model2vec/tokenizer.json"
printf '[]' >"$BUNDLE/taxonomy-schemas.json"

# ── the compiler stub ───────────────────────────────────────────────────────
# A real Mach-O, because three things downstream read one: the linkage audit
# (otool -L), the bundle's version floor (otool -l LC_BUILD_VERSION) and
# codesign. And it answers --check-type-source, so the read-back's run leg
# executes a binary out of the packaged artifact instead of being skipped.
# THE ENGINE: the real pinned DuckDB CLI, not a stand-in, because package.sh
# refuses any executable whose sha256 is not the pin. Fetched once per run
# (16.3 MB for arm64), or taken from BRIGHTFIELD_DUCKDB_CLI when the caller
# already has a directory scripts/fetch-duckdb-cli.sh wrote.
ENGINE="${BRIGHTFIELD_DUCKDB_CLI:-}"
if [ -z "$ENGINE" ]; then
	ENGINE="$TMP/engine"
	"$HERE/fetch-duckdb-cli.sh" "$TARGET" "$ENGINE" >/dev/null || {
		echo "package-artifact-staging-selftest: could not fetch the pinned DuckDB CLI" >&2
		exit 1
	}
fi
ENGINE_ARG="$ENGINE"

STUB="$TMP/stub"
mkdir -p "$STUB"
cat >"$TMP/stub-main.c" <<'C'
#include <stdio.h>
#include <string.h>
int main(int argc, char **argv) {
  if (argc > 1 && strcmp(argv[1], "--check-type-source") == 0) {
    printf("STUB-RAN-THE-PACKAGED-BINARY\n");
    return 0;
  }
  return 64;
}
C
cat >"$STUB/cargo" <<SH
#!/bin/sh
[ "\$1" = "build" ] || { echo "stub cargo: unexpected \$*" >&2; exit 64; }
mkdir -p target/release
exec cc -o target/release/brightfield-shell "$TMP/stub-main.c"
SH
chmod +x "$STUB/cargo"

# ── a throwaway copy of the checkout ────────────────────────────────────────
# Packaging writes into dist/ and target/ of whatever tree it is run from, and a
# self-test must not decide what is in a developer's target/. The copy is of the
# WORKING TREE, not of HEAD: the point of the mutation cases below is to change
# scripts/package.sh and see the difference, which a copy of the last commit
# would not show.
COPY="$TMP/checkout"
copy_checkout() {
	rm -rf "$COPY"
	mkdir -p "$COPY"
	rsync -a --exclude .git --exclude target --exclude dist "$ROOT/" "$COPY/"
}

run_packaging() { # run_packaging -> exit status, log in $out
	(
		cd "$COPY" || exit 1
		PATH="$STUB:$PATH" BRIGHTFIELD_FINETYPE_BUNDLE="$BUNDLE" \
			BRIGHTFIELD_DUCKDB_CLI="$ENGINE_ARG" \
			./scripts/package.sh "v${CRATE_VERSION}" "$TARGET"
	) >"$out" 2>&1
}

read_back() { # read_back ARTIFACT -> exit status, log appended to $out
	(cd "$COPY" && ./scripts/check-artifact-type-source.sh "dist/${NAME}.${1}" "$TARGET") \
		>>"$out" 2>&1
}

# mutate OLD NEW — one occurrence, in the copy's scripts/package.sh.
# attach_image DMG MOUNTPOINT — hdiutil attach, retried. Measured on a
# developer Mac: an attach of a freshly written image failed with "Resource
# temporarily unavailable" in two of two runs of this file, at a different
# attach each time, and left the image attached with no mount point. So a
# failed attempt detaches whatever device the image left behind before the
# next one, and four failures in a row are a failure.
attach_image() {
	local n=0 dev img
	img="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
	until hdiutil attach -nobrowse -noverify -readonly -mountpoint "$2" "$1" >/dev/null; do
		for dev in $(hdiutil info | awk -v p="$img" '
			$1 == "image-path" { hit = (index($0, p) > 0); next }
			hit && $1 ~ /^\/dev\/disk[0-9]+$/ { print $1; hit = 0 }'); do
			hdiutil detach "$dev" -force -quiet >/dev/null 2>&1 || true
		done
		n=$((n + 1))
		[ "$n" -lt 4 ] || return 1
		sleep 2
	done
}

# engine_in KIND -> 0 when the artifact holds the pinned CLI at the path
# `staged_engine_beside` in crates/brightfield-shell/src/run.rs reads for that
# layout, and it runs as v1.5.2; the reason it does not is appended to $out.
LOOK="$TMP/look"
engine_in() {
	local kind="$1" root rel status=0 version
	rm -rf "$LOOK"
	mkdir -p "$LOOK"
	case "$kind" in
	tar.gz)
		tar -xzf "$COPY/dist/${NAME}.tar.gz" -C "$LOOK" || return 1
		root="$LOOK/${NAME}"
		rel="engine/duckdb"
		;;
	dmg)
		attach_image "$COPY/dist/${NAME}.dmg" "$LOOK" || return 1
		root="$LOOK/Brightfield.app"
		rel="Contents/Helpers/duckdb"
		;;
	esac
	if [ ! -f "$root/$rel" ]; then
		echo "the ${kind} carries no engine at ${rel}" >>"$out"
		status=1
	elif ! "$HERE/fetch-duckdb-cli.sh" --check "$TARGET" "$(dirname "$root/$rel")" >>"$out" 2>&1; then
		status=1
	else
		version="$("$root/$rel" --version 2>&1)"
		case "$version" in
		"v1.5.2 "*) ;;
		*)
			echo "the ${kind}'s engine answered --version with: ${version}" >>"$out"
			status=1
			;;
		esac
	fi
	if [ "$kind" = dmg ]; then
		local n=0
		until hdiutil detach "$LOOK" -quiet >/dev/null 2>&1; do
			n=$((n + 1))
			[ "$n" -lt 5 ] || {
				hdiutil detach "$LOOK" -force -quiet >/dev/null 2>&1 || true
				break
			}
			sleep 1
		done
	fi
	return "$status"
}

mutate() {
	local file="$COPY/scripts/package.sh" count
	count=$(grep -cF -- "$1" "$file")
	if [ "$count" -ne 1 ]; then
		echo "  WRONG the anchor '$1' appears ${count} times in scripts/package.sh, not once"
		failures=$((failures + 1))
		return 1
	fi
	python3 - "$file" "$1" "$2" <<'PY'
import sys, pathlib
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
text = pathlib.Path(path).read_text()
assert text.count(old) == 1
pathlib.Path(path).write_text(text.replace(old, new, 1))
PY
}

echo "== packaging as it stands"
copy_checkout
if ! run_packaging; then
	echo "  FAIL packaging did not complete:"
	sed 's/^/       /' "$out"
	exit 1
fi
for kind in tar.gz dmg; do
	if read_back "$kind"; then
		echo "  ok   the ${kind} carries a type source the packaged binary loads"
	else
		echo "  FAIL the ${kind} does not:"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
done

# THE STRUCTURAL PIN OF THIS FILE. Everything above is satisfied by a read-back
# that decided not to run the binary — and then the two cases above would be
# reading a file tree, which is the reading that already existed. The marker is
# printed by the stub binary and by nothing else.
for kind in tar.gz dmg; do
	if engine_in "$kind"; then
		echo "  ok   the ${kind} carries the pinned DuckDB CLI where the runner looks, and it runs"
	else
		echo "  FAIL the ${kind} does not carry the pinned DuckDB CLI where the runner looks:"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
done

if grep -q 'STUB-RAN-THE-PACKAGED-BINARY' "$out"; then
	echo "  ok   the packaged binary was executed out of the artifact"
else
	echo "  FAIL the binary was never run, so both cases above read a file tree only:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

# ── the ways staging has been broken ────────────────────────────────────────
# Each is applied to scripts/package.sh in the copy, packaging is re-run, and the
# artifact whose staging call was removed must be REFUSED, naming the path the
# reader opens. The other artifact is required to still pass: a mutation that
# broke both would prove nothing about which call the check reads.

broken_case() { # broken_case NAME OLD NEW REFUSED_KIND REFUSED_NEEDLE OK_KIND
	local name="$1" old="$2" new="$3" bad="$4" needle="$5" good="$6"
	copy_checkout
	mutate "$old" "$new" || return
	if ! run_packaging; then
		echo "  FAIL ${name}: packaging itself broke, so the read-back was never reached:"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if read_back "$bad"; then
		echo "  FAIL ${name}: the ${bad} still passed"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if ! grep -qF -- "$needle" "$out"; then
		echo "  FAIL ${name}: refused without naming ${needle}"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if ! read_back "$good"; then
		echo "  FAIL ${name}: the ${good} broke too, so this says nothing about which call is read"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

echo "== packaging with the type source dropped from one artifact"

broken_case "the app bundle's staging call is deleted" \
	'    stage_finetype "$APP/Contents/Resources/finetype"' \
	'    :' \
	dmg "carries no type source at Brightfield.app/Contents/Resources/finetype" tar.gz

broken_case "the app bundle's staging call is reduced to a mkdir" \
	'    stage_finetype "$APP/Contents/Resources/finetype"' \
	'    mkdir -p "$APP/Contents/Resources/finetype"' \
	dmg "finetype.duckdb_extension" tar.gz

broken_case "the tarball's staging call is deleted" \
	'stage_finetype "$STAGE/finetype"' \
	':' \
	tar.gz "carries no type source at finetype" dmg

# broken_engine_case NAME OLD MISSING_KIND NEEDLE PRESENT_KIND — one staging
# call deleted; the artifact it fed must be refused naming the path, and the
# other must still pass, so the case says which call each artifact reads.
broken_engine_case() {
	local name="$1" old="$2" bad="$3" needle="$4" good="$5"
	copy_checkout
	mutate "$old" ':' || return
	if ! run_packaging; then
		echo "  FAIL ${name}: packaging itself broke, so the artifacts were never read:"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if engine_in "$bad"; then
		echo "  FAIL ${name}: the ${bad} still passed"
		failures=$((failures + 1))
		return
	fi
	if ! grep -qF -- "$needle" "$out"; then
		echo "  FAIL ${name}: refused without naming ${needle}"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if ! engine_in "$good"; then
		echo "  FAIL ${name}: the ${good} broke too, so this says nothing about which call is read"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

echo "== packaging with the engine dropped from one artifact"

broken_engine_case "the app bundle's engine staging call is deleted" \
	'    stage_engine "$APP/Contents/Helpers/duckdb"' \
	dmg "carries no engine at Contents/Helpers/duckdb" tar.gz

broken_engine_case "the tarball's engine staging call is deleted" \
	'stage_engine "$STAGE/engine/duckdb"' \
	tar.gz "carries no engine at engine/duckdb" dmg

echo "== packaging handed a duckdb that is not the pin"
copy_checkout
mkdir -p "$TMP/impostor"
printf '#!/bin/sh\necho "v1.5.2 (impostor)"\n' >"$TMP/impostor/duckdb"
chmod +x "$TMP/impostor/duckdb"
ENGINE_ARG="$TMP/impostor"
if run_packaging; then
	echo "  FAIL packaging staged an engine whose sha256 is not the pin"
	failures=$((failures + 1))
elif ! grep -qF "is not the pinned DuckDB v1.5.2 CLI for ${TARGET}" "$out"; then
	echo "  FAIL packaging refused, but not for the engine:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
elif [ -e "$COPY/target/release/brightfield-shell" ]; then
	echo "  FAIL the engine was refused only after the build ran"
	failures=$((failures + 1))
else
	echo "  ok   an engine that is not the pinned CLI is refused before the build"
fi
ENGINE_ARG="$ENGINE"

echo
if [ "$failures" -ne 0 ]; then
	echo "package-artifact-staging-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "package-artifact-staging-selftest: a real packaging run stages the type source and the engine into both artifacts; dropping any staging call, or handing it a duckdb that is not the pin, is refused."
