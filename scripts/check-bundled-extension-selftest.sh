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

# The 512-byte DuckDB metadata trailer, written by the shared fixture maker so
# these offsets live in one file rather than in each self-test that needs them.
make_extension() {
  "${HERE}/fixture-extension.py" "$@"
}

# The optional second encoder the published model names in its own config.json.
# It sits BESIDE model2vec/ and does not replace it: FineType's loader opens
# model2vec/ unconditionally and never reads its name from the config, while
# value_embed_model names an extra encoder a single-encoder model does without.
# A revision of the check that swapped the two — dropping the mandatory
# directory because the config is silent about it, requiring the optional one
# because the config names it — produced a bundle that passed every file check
# and would not load.
EMBED=value_model2vec

# A complete, well-formed bundle at $1.
make_bundle() {
  local dir="$1" platform="${2:-osx_arm64}" abi="${3:-C_STRUCT}" ext_version="${4:-0.6.56}"
  rm -rf "$dir"
  mkdir -p "$dir/model/model2vec" "$dir/model/$EMBED"
  make_extension "$dir/finetype.duckdb_extension" "$platform" v1.2.0 "$ext_version" "$abi"
  printf 'safetensors'   > "$dir/model/model.safetensors"
  printf '{"value_embed_model": "%s"}' "$EMBED" > "$dir/model/config.json"
  printf '{}'            > "$dir/model/label_map.json"
  printf 'safetensors'   > "$dir/model/model2vec/model.safetensors"
  printf '{}'            > "$dir/model/model2vec/tokenizer.json"
  printf 'safetensors'   > "$dir/model/$EMBED/model.safetensors"
  printf '{}'            > "$dir/model/$EMBED/tokenizer.json"
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
               model/label_map.json model/model2vec/model.safetensors \
               model/model2vec/tokenizer.json taxonomy-schemas.json; do
  make_bundle "$TMP/missing"
  rm "$TMP/missing/$missing"
  expect_fail "without ${missing}" "carries no ${missing}" "$TMP/missing"
done

echo "== a bundle whose value-embedding directory is not the one its config names"
# The extension opens the directory model/config.json names in
# `value_embed_model`. A bundle carrying the files under some other name is one
# the extension cannot use, and it is what a fetch that renamed the directory
# to suit a hardcoded check would produce.
make_bundle "$TMP/renamed"
rm -rf "$TMP/renamed/model/$EMBED"
mv "$TMP/renamed/model/model2vec" "$TMP/renamed/model/$EMBED"
expect_fail "the mandatory encoder renamed to the one the config names" \
  "carries no model/model2vec/model.safetensors" "$TMP/renamed"

make_bundle "$TMP/halfembed"
rm "$TMP/halfembed/model/$EMBED/tokenizer.json"
expect_fail "the embedding tokenizer missing" \
  "carries no model/${EMBED}/tokenizer.json" "$TMP/halfembed"

# ABSENT IS VALID and this case is a correction. FineType's loader returns "no
# value encoder" for a model whose config does not name one, so a check that
# refused it was asserting a property the product does not have — and would
# have refused every single-encoder model FineType ever ships.
make_bundle "$TMP/noembed"
printf '{"n_classes": 1}' > "$TMP/noembed/model/config.json"
rm -rf "$TMP/noembed/model/$EMBED"
expect_pass "a single-encoder model, whose config names no value_embed_model" "$TMP/noembed"

# Named and missing is not valid: the loader goes looking for what the config
# names.
make_bundle "$TMP/named-absent"
rm -rf "$TMP/named-absent/model/$EMBED"
expect_fail "a config naming an encoder the bundle does not carry" \
  "carries no model/${EMBED}/model.safetensors" "$TMP/named-absent"

make_bundle "$TMP/escape"
printf '{"value_embed_model": "../elsewhere"}' > "$TMP/escape/model/config.json"
expect_fail "a value_embed_model that is a path" "which is
a path rather than a directory name" "$TMP/escape"
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
# label_map.json and not config.json: the value_embed_model read happens before
# the manifest comparison, so a scrambled config is refused for naming no
# embedding directory and this case would pass on the wrong message.
printf 'different bytes entirely' > "$TMP/manifest/model/label_map.json"
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

echo "== an extension that is not the pinned FineType release"
# The third argument is what scripts/package.sh passes from
# packaging/finetype-pin.env. Without this comparison the pin is a string two
# scripts agree about while the staged bytes come from wherever, which is the
# defect a declaration nothing measures always has.
make_bundle "$TMP/pinned" osx_arm64 C_STRUCT 0.6.58
expect_pass "the pinned version, tag spelling" "$TMP/pinned" osx_arm64 v0.6.58
expect_pass "the pinned version, bare spelling" "$TMP/pinned" osx_arm64 0.6.58
expect_fail "one patch release behind the pin" "is stamped version '0.6.58'" \
  "$TMP/pinned" osx_arm64 v0.6.59
expect_fail "a different minor entirely" "packaging/finetype-pin.env declares 'v0.7.0'" \
  "$TMP/pinned" osx_arm64 v0.7.0
# No version argument is still a pass: scripts/verify-airgapped.sh passes none,
# because the checkout reading a built artifact is not necessarily the checkout
# whose pin built it.
expect_pass "no version declared at all" "$TMP/pinned" osx_arm64

echo
if [ "$fails" -ne 0 ]; then
  echo "check-bundled-extension-selftest: ${fails} case(s) did not behave as required." >&2
  exit 1
fi
echo "check-bundled-extension-selftest: the guard passes what it should and refuses what it should."
