#!/usr/bin/env bash
# Prove a REAL scripts/package.sh run puts the type source inside every artifact
# it produces, at the path that artifact's own reader opens.
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
			./scripts/package.sh "v${CRATE_VERSION}" "$TARGET"
	) >"$out" 2>&1
}

read_back() { # read_back ARTIFACT -> exit status, log appended to $out
	(cd "$COPY" && ./scripts/check-artifact-type-source.sh "dist/${NAME}.${1}" "$TARGET") \
		>>"$out" 2>&1
}

# mutate OLD NEW — one occurrence, in the copy's scripts/package.sh.
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

echo
if [ "$failures" -ne 0 ]; then
	echo "package-artifact-staging-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "package-artifact-staging-selftest: a real packaging run stages the type source into both artifacts, and dropping either call is refused."
