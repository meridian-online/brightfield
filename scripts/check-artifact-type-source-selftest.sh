#!/usr/bin/env bash
# Prove scripts/check-artifact-type-source.sh reddens on the artifact that
# actually shipped.
#
# THE FAILURE THIS IS ABOUT IS ONE LEVEL UP FROM A BAD BUNDLE. A card recorded
# the FineType extension and its model as shipping in the artifact, and three
# independent reviewers agreed, each having assembled a real bundle by hand and
# watched the checks read it. Every one of those readings was of a fixture. The
# path a release takes was what nobody exercised, and along that path
# `stage_finetype` returned at its first line and every artifact was packaged
# with nothing in it.
#
# So the fixtures here are ARTIFACTS, not bundles: a tarball built the way
# scripts/package.sh builds one, and on macOS a disk image built the way it
# builds one. What is asserted is what the check says about the unpacked
# artifact — including the case that matters most, which is the bundle being
# absent, and the case next to it, which is the bundle being present one
# directory away from where the executable looks.
#
# WHERE EACH CASE RUNS. The tarball cases need only tar and run everywhere,
# including the toolchain-free hygiene runner. The disk image cases need
# `hdiutil` and therefore macOS; test.yml runs this file on macos-15 for
# exactly that reason, so no case here is left unexercised in CI. On Linux the
# image cases are reported as not applicable rather than counted as passes.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$HERE/check-artifact-type-source.sh"

TAG="$("$HERE/finetype-pin.sh")"

# The fixtures are built for THIS machine, so the run leg is exercised on every
# runner rather than only on the architecture a hard-coded triple happened to
# name. The check decides whether to run the binary by comparing its target
# argument against the host, so a fixture for another triple would silently
# turn the most important case in this file into a no-op.
TARGET="$("$CHECK" --print-host-target)"
PLATFORM="$("$HERE/duckdb-platform.sh" "$TARGET")"

# A target this machine is definitely not, for the cross-compile case.
if [ "$TARGET" = "aarch64-apple-darwin" ]; then
	FOREIGN="x86_64-apple-darwin"
else
	FOREIGN="aarch64-apple-darwin"
fi
FOREIGN_PLATFORM="$("$HERE/duckdb-platform.sh" "$FOREIGN")"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
out="$TMP/out"

# A bundle of the shape scripts/check-bundled-extension.sh accepts, with the
# staged manifest scripts/package.sh writes over it — because the check reads
# that manifest, and an artifact whose bundle changed after packaging is one of
# the things it is supposed to catch.
make_bundle() { # make_bundle DIR [PLATFORM] [VERSION]
	local dir="$1" platform="${2:-$PLATFORM}" version="${3:-${TAG#v}}"
	rm -rf "$dir"
	# BOTH encoder directories, and they are not interchangeable. model2vec/ is
	# opened unconditionally by FineType's loader and is named nowhere in the
	# model's config; value_model2vec/ is the optional second encoder the config
	# does name. A fixture carrying only the second is one that assembles and
	# does not load.
	mkdir -p "$dir/model/model2vec" "$dir/model/value_model2vec"
	"$HERE/fixture-extension.py" "$dir/finetype.duckdb_extension" "$platform" v1.2.0 "$version" C_STRUCT
	printf 'weights' >"$dir/model/model.safetensors"
	printf '{"value_embed_model": "value_model2vec"}' >"$dir/model/config.json"
	printf '{}' >"$dir/model/label_map.json"
	printf 'weights' >"$dir/model/model2vec/model.safetensors"
	printf '{}' >"$dir/model/model2vec/tokenizer.json"
	printf 'weights' >"$dir/model/value_model2vec/model.safetensors"
	printf '{}' >"$dir/model/value_model2vec/tokenizer.json"
	printf '[]' >"$dir/taxonomy-schemas.json"
	(cd "$dir" && find . -type f ! -name bundle-manifest.sha256 | sed 's|^\./||' | LC_ALL=C sort |
		xargs shasum -a 256 >bundle-manifest.sha256)
}

# A stand-in for the packaged binary, answering --check-type-source with a
# chosen exit code. The real one loads a DuckDB extension and a 17 MB model;
# what this file is about is what the check does with each answer.
make_exe() { # make_exe PATH EXITCODE
	cat >"$1" <<SH
#!/bin/sh
[ "\$1" = "--check-type-source" ] || { echo "unexpected argument: \$1" >&2; exit 64; }
echo "STUB-RAN-THE-BINARY: --check-type-source, exit ${2}"
exit ${2}
SH
	chmod +x "$1"
}

# make_tarball NAME [--no-bundle|--bundle-at REL] [--exit N] [--platform P] [--version V] [--no-exe]
# Builds dist-shaped output at $TMP/<NAME>.tar.gz and echoes the path.
make_tarball() {
	local name="$1"
	shift
	local bundle_at="finetype" exitcode=0 platform="$PLATFORM" version="${TAG#v}" with_exe=1
	while [ "$#" -gt 0 ]; do
		case "$1" in
		--no-bundle) bundle_at=""; shift ;;
		--bundle-at) bundle_at="$2"; shift 2 ;;
		--exit) exitcode="$2"; shift 2 ;;
		--platform) platform="$2"; shift 2 ;;
		--version) version="$2"; shift 2 ;;
		--no-exe) with_exe=0; shift ;;
		*) echo "make_tarball: unknown $1" >&2; return 1 ;;
		esac
	done
	local stage="$TMP/stage-$name/brightfield-v9.9.9-$TARGET"
	rm -rf "$TMP/stage-$name"
	mkdir -p "$stage"
	[ "$with_exe" -eq 1 ] && make_exe "$stage/brightfield" "$exitcode"
	if [ -n "$bundle_at" ]; then
		mkdir -p "$(dirname "$stage/$bundle_at")"
		make_bundle "$stage/$bundle_at" "$platform" "$version"
	fi
	tar -czf "$TMP/$name.tar.gz" -C "$TMP/stage-$name" "brightfield-v9.9.9-$TARGET"
	printf '%s\n' "$TMP/$name.tar.gz"
}

expect_pass() {
	local name="$1"
	shift
	if "$CHECK" "$@" >"$out" 2>&1; then
		echo "  ok   ${name}"
	else
		echo "  FAIL ${name}: expected a pass, got a refusal"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi
}

expect_fail() {
	local name="$1" needle="$2"
	shift 2
	if "$CHECK" "$@" >"$out" 2>&1; then
		echo "  FAIL ${name}: expected a refusal, got a pass"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if ! grep -qF -- "$needle" "$out"; then
		echo "  FAIL ${name}: refused without saying ${needle}"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

echo "== a tarball that carries a working type source"
good="$(make_tarball good)"
expect_pass "the bundle is read off the unpacked artifact and the binary loads it" \
	"$good" "$TARGET"

# THE STRUCTURAL PIN OF THIS WHOLE FILE. Nothing above asked for the binary to
# be run; the check decided to, because the artifact's target is this machine's.
# If it ever stops deciding that, every "the binary reports X" case below turns
# into a silent no-op that still reports ok — which is exactly what happened
# when the run was a caller-supplied flag and a review pass mutated it away.
if grep -q 'STUB-RAN-THE-BINARY' "$out"; then
	echo "  ok   the packaged binary was executed without anything asking for it"
else
	echo "  FAIL the binary was never run, so every run case below proves nothing:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

echo "== a tarball that carries none"
# THE CASE THAT SHIPPED. Everything else in this file is a variation on it.
none="$(make_tarball none --no-bundle)"
expect_fail "no type source in the artifact at all" "carries no type source at finetype" \
	"$none" "$TARGET"
expect_fail "and running the binary does not make it pass either" \
	"carries no type source at finetype" "$none" "$TARGET"

echo "== a tarball whose bundle is not where the binary looks"
# `semantic::bundle_beside` consults <exe dir>/finetype and
# <exe dir>/../Resources/finetype and nothing else. A bundle one directory
# across is an artifact carrying a classifier it will never open, and a check
# that searched the artifact for the directory by name would call it present.
across="$(make_tarball across --bundle-at "resources/finetype")"
expect_fail "the bundle staged one directory across" "carries no type source at finetype" \
	"$across" "$TARGET"

nested="$(make_tarball nested --bundle-at "examples/finetype")"
expect_fail "the bundle staged under examples/" "carries no type source at finetype" \
	"$nested" "$TARGET"

echo "== a tarball whose bundle would not load"
# The wrong platform is DERIVED from the host, not written down. It was
# `linux_amd64`, which is another platform on a Mac and is the host's own on
# the ubuntu runner this file also runs on — so the case passed locally and
# passed in CI for opposite reasons, proving the mismatch on one and nothing on
# the other.
crossbuilt="$(make_tarball crossbuilt --platform "$FOREIGN_PLATFORM")"
expect_fail "an extension for another platform, inside the packaged artifact" \
	"built for '${FOREIGN_PLATFORM}'" "$crossbuilt" "$TARGET"

stale="$(make_tarball stale --version 0.0.1)"
expect_fail "a bundle from a release the pin does not name" \
	"packaging/finetype-pin.env declares" "$stale" "$TARGET"

# The manifest scripts/package.sh wrote over the staged copy, contradicted.
# What this catches is an artifact whose bundle changed after packaging — a
# partial unpack, a stale file — which no other check here can see.
tampered="$(make_tarball tampered)"
rm -rf "$TMP/untar" && mkdir -p "$TMP/untar"
tar -xzf "$tampered" -C "$TMP/untar"
printf 'different bytes entirely' >"$TMP/untar/brightfield-v9.9.9-$TARGET/finetype/model/label_map.json"
tar -czf "$tampered" -C "$TMP/untar" "brightfield-v9.9.9-$TARGET"
expect_fail "a bundle that changed after packaging" "does not match its own manifest" \
	"$tampered" "$TARGET"

echo "== an artifact the binary cannot be run out of"
noexe="$(make_tarball noexe --no-exe)"
expect_fail "no executable in the tarball" "carries no executable" "$noexe" "$TARGET"

echo "== what the packaged binary itself reports"
# Exit 2 is the binary saying it found no bundle. With a bundle visibly in the
# artifact that is a staging-path defect, and it gets its own message because
# it is the one failure a person would otherwise chase in the wrong file.
saysnone="$(make_tarball saysnone --exit 2)"
expect_fail "the binary reports no bundle while the artifact carries one" \
	"staged somewhere the executable does not look" "$saysnone" "$TARGET"

broken="$(make_tarball broken --exit 1)"
expect_fail "the bundled type source does not come up" "did not come up (exit 1)" \
	"$broken" "$TARGET"

echo "== an artifact for a machine that cannot execute it"
# The release matrix cross-compiles x86_64 on an arm64 runner. The check must
# read the bundle and DECLINE to run the binary, saying which two triples
# disagree — and a stub that would fail if run must not fail the case, which is
# what proves the decline is real rather than the run quietly passing.
foreign="$(make_tarball foreign --platform "$FOREIGN_PLATFORM" --exit 1)"
expect_pass "a cross-compiled artifact is read and not executed" "$foreign" "$FOREIGN"
if grep -q 'STUB-RAN-THE-BINARY' "$out"; then
	echo "  FAIL the binary was executed for a target this machine is not:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
elif grep -q "this machine is ${TARGET}" "$out"; then
	echo "  ok   it declines to run and names both triples"
else
	echo "  FAIL it neither ran nor said why it did not:"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

echo "== an artifact of a shape this check does not know"
printf 'not an artifact' >"$TMP/thing.zip"
expect_fail "a .zip" "not an artifact this script knows" "$TMP/thing.zip" "$TARGET"
expect_fail "a path that is not there" "no such artifact" "$TMP/absent.tar.gz" "$TARGET"
expect_fail "a target with no DuckDB platform name" "no DuckDB platform name known" \
	"$good" "x86_64-pc-windows-msvc"

echo "== the disk image"
if [ "$(uname -s)" != "Darwin" ]; then
	echo "  --   not applicable on $(uname -s): hdiutil is macOS-only, and test.yml"
	echo "       runs this file on macos-15 so these cases are covered there"
else
	command -v hdiutil >/dev/null 2>&1 || {
		echo "selftest: hdiutil is missing on a Darwin host" >&2
		exit 1
	}
	# make_image NAME [--no-bundle]
	make_image() {
		local name="$1" nobundle="${2:-}"
		local stage="$TMP/image-$name"
		local app="$stage/Brightfield.app"
		rm -rf "$stage"
		mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
		make_exe "$app/Contents/MacOS/brightfield" 0
		[ "$nobundle" = "--no-bundle" ] || make_bundle "$app/Contents/Resources/finetype"
		local kb
		kb=$(du -sk "$stage" | awk '{print $1}')
		rm -f "$TMP/$name.dmg"
		hdiutil create -volname Brightfield -srcfolder "$stage" \
			-size "$((kb + 65536))k" -format ULFO -ov "$TMP/$name.dmg" >/dev/null
		printf '%s\n' "$TMP/$name.dmg"
	}

	img_good="$(make_image good)"
	expect_pass "the app bundle's Contents/Resources/finetype is read and loaded" \
		"$img_good" "$TARGET"
	if grep -q 'STUB-RAN-THE-BINARY' "$out"; then
		echo "  ok   the bundled binary was executed off the mounted image"
	else
		echo "  FAIL the image's binary was never run:"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
	fi

	img_none="$(make_image none --no-bundle)"
	expect_fail "an image whose app carries no type source" \
		"carries no type source at Brightfield.app/Contents/Resources/finetype" \
		"$img_none" "$TARGET"
fi

echo
if [ "$failures" -ne 0 ]; then
	echo "check-artifact-type-source-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "check-artifact-type-source-selftest: the check reads the packaged artifact and refuses an empty one."
