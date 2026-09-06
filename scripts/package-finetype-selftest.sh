#!/usr/bin/env bash
# Regression test for the type-source refusals in scripts/package.sh.
#
# scripts/check-bundled-extension-selftest.sh already proves the CHECK reddens.
# This proves PACKAGING reddens, which is a different claim and the one the
# acceptance criterion is about: a deliberate mismatch must fail the packaging
# run rather than produce an artifact that loads nothing. A check that is
# correct and never reached is the failure this file exists to keep out, and it
# is not hypothetical here — for as long as no workflow set
# BRIGHTFIELD_FINETYPE_BUNDLE, the whole block was skipped on every release.
#
# It also proves the ORDERING. The bundle is read before the compiler is
# invoked, so a bad bundle costs a second rather than a ten-minute release
# build — and, more to the point, so this file can run on the toolchain-free
# hygiene runner at all. Every failing case asserts `== build` never appeared.
#
# THE PIN CASE IS THE SUBTLE ONE. It overrides the pin with a tag the fixture
# deliberately does not carry, so a package.sh that compared a hardcoded
# version instead of reading packaging/finetype-pin.env would pass the fixture
# and fail this. `scripts/package.sh --print-finetype-pin` cannot show that:
# a print mode can read the pin correctly while the staging path beside it
# reads a literal.
#
# No real build runs. rustc is stubbed to report a host and cargo to fail, so
# every case stops at or just after the block under test with no dist/ output
# and no compiler.
set -uo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
PKG="$ROOT/scripts/package.sh"

CRATE_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/crates/brightfield-shell/Cargo.toml" | head -1)"
TAG="$("$HERE/finetype-pin.sh")"

failures=0
TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT
out="$TMP/out"

# The stubs. rustc must SUCCEED and name a host — package.sh reads it under
# `set -o pipefail` and would die on the assignment otherwise, before reaching
# anything this file is about. cargo must fail, which is how a case that gets
# past the block under test stops without building.
STUB="$TMP/stub"
mkdir -p "$STUB"
printf '#!/bin/sh\necho "host: aarch64-apple-darwin"\n' >"$STUB/rustc"
printf '#!/bin/sh\nexit 1\n' >"$STUB/cargo"
chmod +x "$STUB/rustc" "$STUB/cargo"

# make_bundle DIR [PLATFORM] [EXT_VERSION] [ABI]
make_bundle() {
	local dir="$1" platform="${2:-osx_arm64}" version="${3:-${TAG#v}}" abi="${4:-C_STRUCT}"
	rm -rf "$dir"
	# value_model2vec, the name the published model's own config.json gives the
	# directory — not the "model2vec" literal the check used to require.
	mkdir -p "$dir/model/value_model2vec"
	"$HERE/fixture-extension.py" "$dir/finetype.duckdb_extension" "$platform" v1.2.0 "$version" "$abi"
	printf 'weights' >"$dir/model/model.safetensors"
	printf '{"value_embed_model": "value_model2vec"}' >"$dir/model/config.json"
	printf '{}' >"$dir/model/label_map.json"
	printf 'weights' >"$dir/model/value_model2vec/model.safetensors"
	printf '{}' >"$dir/model/value_model2vec/tokenizer.json"
	printf '[]' >"$dir/taxonomy-schemas.json"
}

# run_package TARGET [ENV=VAL ...] — package.sh with the stubs in front.
run_package() {
	local target="$1"
	shift
	(
		cd "$ROOT" || exit 1
		PATH="$STUB:$PATH" env "$@" "$PKG" "v${CRATE_VERSION}" "$target"
	) >"$out" 2>&1
}

# expect_refusal NAME NEEDLE TARGET [ENV=VAL ...]
expect_refusal() {
	local name="$1" needle="$2"
	shift 2
	if run_package "$@"; then
		echo "  FAIL ${name}: packaging succeeded"
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
	# The ordering claim, asserted on every failing case rather than once.
	if grep -q '^== build' "$out"; then
		echo "  FAIL ${name}: the compiler was invoked before the bundle was read"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

# expect_reaches_build NAME TARGET [ENV=VAL ...] — the bundle was accepted.
expect_reaches_build() {
	local name="$1"
	shift
	run_package "$@"
	if ! grep -q '^== build' "$out"; then
		echo "  FAIL ${name}: packaging stopped before the build"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	if grep -q 'refusing to stage' "$out"; then
		echo "  FAIL ${name}: the guard cried wolf"
		sed 's/^/       /' "$out"
		failures=$((failures + 1))
		return
	fi
	echo "  ok   ${name}"
}

echo "== a bundle packaging must refuse"
make_bundle "$TMP/wrongarch" linux_amd64
expect_refusal "an extension built for another platform" "built for 'linux_amd64'" \
	aarch64-apple-darwin "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/wrongarch"

# The cross-packaging case in its real shape: the host's own extension staged
# into the artifact for the OTHER darwin target. It works on the machine that
# built it and on no machine it was built for.
make_bundle "$TMP/hostarch" osx_arm64
expect_refusal "the host's extension in the x86_64 artifact" "built for 'osx_arm64'" \
	x86_64-apple-darwin "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/hostarch"

make_bundle "$TMP/cpp" osx_arm64 "${TAG#v}" CPP
expect_refusal "an unstable-C-API build" "not C_STRUCT" \
	aarch64-apple-darwin "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/cpp"

make_bundle "$TMP/gutted" osx_arm64
rm "$TMP/gutted/taxonomy-schemas.json"
expect_refusal "a bundle missing its taxonomy catalogue" "carries no taxonomy-schemas.json" \
	aarch64-apple-darwin "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/gutted"

expect_refusal "a target with no DuckDB platform name" "no DuckDB platform name known" \
	x86_64-pc-windows-msvc "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/hostarch"

echo "== a bundle that is not the pinned FineType release"
# The pin is overridden and the fixture keeps the version the real pin names,
# so this reddens only if the STAGING path reads the pin. A package.sh
# comparing a hardcoded version would see the fixture agree and let it through.
printf 'FINETYPE_TAG=v9.9.9\n' >"$TMP/other-pin.env"
make_bundle "$TMP/pinned" osx_arm64 "${TAG#v}"
expect_refusal "a bundle from a release the pin does not name" "declares 'v9.9.9'" \
	aarch64-apple-darwin \
	"BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/pinned" \
	"BRIGHTFIELD_FINETYPE_PIN=$TMP/other-pin.env"

printf 'FINETYPE_TAG=not-a-tag\n' >"$TMP/bad-pin.env"
expect_refusal "a pin that does not parse" "is not a v<major>.<minor>.<patch> tag" \
	aarch64-apple-darwin \
	"BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/pinned" \
	"BRIGHTFIELD_FINETYPE_PIN=$TMP/bad-pin.env"

echo "== a bundle packaging must accept"
expect_reaches_build "a bundle matching the target and the pin" \
	aarch64-apple-darwin "BRIGHTFIELD_FINETYPE_BUNDLE=$TMP/pinned"

# The supported build with no type source: a message, not a failure. This is
# what a contributor's local run produces, and a guard that refused it would
# make scripts/package.sh unusable outside a release.
expect_reaches_build "no bundle at all" aarch64-apple-darwin \
	"BRIGHTFIELD_FINETYPE_BUNDLE="
if ! grep -q 'type source: none' "$out"; then
	echo "  FAIL a build with no bundle did not say so"
	sed 's/^/       /' "$out"
	failures=$((failures + 1))
fi

echo
if [[ "$failures" -ne 0 ]]; then
	echo "package-finetype-selftest: ${failures} case(s) did not behave as required." >&2
	exit 1
fi
echo "package-finetype-selftest: packaging refuses a bundle that would not load, before it builds."
