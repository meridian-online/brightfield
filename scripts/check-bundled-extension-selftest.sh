#!/usr/bin/env bash
# Prove scripts/check-bundled-extension.sh reddens, one defect at a time.
#
# The check it exercises runs in two places that are both expensive and rare —
# a release packaging run and an air-gap verification of a built artifact — so
# without this it would be exercised a handful of times a year, always at the
# moment it is least welcome to be wrong. This runs on every pull request, with
# no toolchain, no network and no real extension: the fixtures are a few
# hundred bytes of synthesised trailer.
#
# Every case asserts BOTH directions. A guard that never passes is as useless
# as one that never fails, and a check whose good case has quietly started
# failing would otherwise show up as "packaging is broken" long after the
# change that broke it.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
CHECK="${HERE}/check-bundled-extension.sh"
TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-bundle-selftest.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

fails=0

# Write a 512-byte DuckDB metadata trailer onto stdout, appended to some
# plausible library body. Field order is LAST-FIRST: fields 8..6 empty, then
# ABI, extension version, DuckDB version, platform, magic — then 256 zero bytes
# where a signature would go on a signed build.
#
# Built with python3 rather than printf/dd because the fields are NUL-padded
# and shell cannot carry a NUL through a variable.
make_extension() {
  local out="$1" platform="$2" duckdb_version="$3" ext_version="$4" abi="$5"
  python3 - "$out" "$platform" "$duckdb_version" "$ext_version" "$abi" <<'PY'
import sys
out, platform, duckdb_version, ext_version, abi = sys.argv[1:6]
pad = lambda s: s.encode().ljust(32, b"\0")
body = b"a plausible shared library body" * 40
trailer = b"\0" * 96
trailer += pad(abi) + pad(ext_version) + pad(duckdb_version) + pad(platform) + pad("4")
trailer += b"\0" * 256
assert len(trailer) == 512, len(trailer)
open(out, "wb").write(body + trailer)
PY
}

# A complete, well-formed bundle at $1.
make_bundle() {
  local dir="$1" platform="${2:-osx_arm64}" abi="${3:-C_STRUCT}"
  rm -rf "$dir"
  mkdir -p "$dir/model/model2vec"
  make_extension "$dir/finetype.duckdb_extension" "$platform" v1.2.0 0.6.56 "$abi"
  printf 'safetensors'   > "$dir/model/model.safetensors"
  printf '{}'            > "$dir/model/config.json"
  printf '{}'            > "$dir/model/label_map.json"
  printf 'safetensors'   > "$dir/model/model2vec/model.safetensors"
  printf '{}'            > "$dir/model/model2vec/tokenizer.json"
  printf '[]'            > "$dir/taxonomy-schemas.json"
}

# expect_pass NAME BUNDLE [PLATFORM]
expect_pass() {
  local name="$1"; shift
  if "$CHECK" "$@" > "$TMP/out" 2>&1; then
    echo "  ok   ${name}"
  else
    echo "  FAIL ${name}: expected a pass, got exit $?"
    sed 's/^/       /' "$TMP/out"
    fails=$((fails + 1))
  fi
}

# expect_fail NAME SUBSTRING BUNDLE [PLATFORM]
expect_fail() {
  local name="$1" needle="$2"; shift 2
  if "$CHECK" "$@" > "$TMP/out" 2>&1; then
    echo "  FAIL ${name}: expected a refusal, got a pass"
    sed 's/^/       /' "$TMP/out"
    fails=$((fails + 1))
    return
  fi
  if ! grep -qF -- "$needle" "$TMP/out"; then
    echo "  FAIL ${name}: refused without saying ${needle}"
    sed 's/^/       /' "$TMP/out"
    fails=$((fails + 1))
    return
  fi
  echo "  ok   ${name}"
}

echo "== the good case"
make_bundle "$TMP/good"
expect_pass "a complete bundle passes" "$TMP/good"
expect_pass "and passes against its own platform" "$TMP/good" osx_arm64

echo "== a bundle that is missing a piece"
for missing in finetype.duckdb_extension model/model.safetensors model/config.json \
               model/label_map.json model/model2vec/tokenizer.json taxonomy-schemas.json; do
  make_bundle "$TMP/missing"
  rm "$TMP/missing/$missing"
  expect_fail "without ${missing}" "carries no ${missing}" "$TMP/missing"
done
expect_fail "a directory that does not exist" "no such directory" "$TMP/absent"

echo "== a bundle that is not self-contained"
make_bundle "$TMP/linked"
mv "$TMP/linked/model/model.safetensors" "$TMP/elsewhere.safetensors"
ln -s "$TMP/elsewhere.safetensors" "$TMP/linked/model/model.safetensors"
# The link RESOLVES here — that is the trap. It would not resolve anywhere
# the artifact is unpacked, and running the artifact on the machine that
# built it can never show that.
[ -f "$TMP/linked/model/model.safetensors" ] || { echo "  FAIL fixture: the link does not resolve"; fails=$((fails + 1)); }
expect_fail "a symlinked model file" "not self-contained" "$TMP/linked"

echo "== a bundle that does not match its own manifest"
make_bundle "$TMP/manifest"
( cd "$TMP/manifest" && find . -type f ! -name bundle-manifest.sha256 | sed 's|^\./||' | LC_ALL=C sort \
    | xargs shasum -a 256 > bundle-manifest.sha256 )
expect_pass "a bundle matching its manifest" "$TMP/manifest"
printf 'different bytes entirely' > "$TMP/manifest/model/config.json"
expect_fail "one file changed after packaging" "does not match its own manifest" "$TMP/manifest"

make_bundle "$TMP/manifest2"
( cd "$TMP/manifest2" && find . -type f ! -name bundle-manifest.sha256 | sed 's|^\./||' | LC_ALL=C sort \
    | xargs shasum -a 256 > bundle-manifest.sha256 )
rm "$TMP/manifest2/taxonomy-schemas.json"
# The existence check fires first, and that is the right message — but the
# manifest would have caught it too, which is what makes a partial unpack
# visible however it is shaped.
expect_fail "a file the manifest names is gone" "carries no taxonomy-schemas.json" "$TMP/manifest2"

make_bundle "$TMP/manifest3"
: > "$TMP/manifest3/bundle-manifest.sha256"
expect_fail "an empty manifest" "names no files" "$TMP/manifest3"

echo "== an extension that would not load"
make_bundle "$TMP/unstamped"
head -c 4096 /dev/zero > "$TMP/unstamped/finetype.duckdb_extension"
expect_fail "an unstamped shared library" "no DuckDB metadata trailer" "$TMP/unstamped"

make_bundle "$TMP/short"
printf 'tiny' > "$TMP/short/finetype.duckdb_extension"
expect_fail "a file shorter than the trailer" "shorter than the metadata trailer" "$TMP/short"

make_bundle "$TMP/cpp" osx_arm64 CPP
expect_fail "an unstable-C-API build" "not C_STRUCT" "$TMP/cpp"

make_bundle "$TMP/wrongarch" linux_amd64
expect_fail "an extension for another platform" "built for 'linux_amd64'" \
  "$TMP/wrongarch" osx_arm64
expect_pass "the same extension against its own platform" "$TMP/wrongarch" linux_amd64

echo
if [ "$fails" -ne 0 ]; then
  echo "check-bundled-extension-selftest: ${fails} case(s) did not behave as required." >&2
  exit 1
fi
echo "check-bundled-extension-selftest: the guard passes what it should and refuses what it should."
