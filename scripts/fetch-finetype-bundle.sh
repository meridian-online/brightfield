#!/usr/bin/env bash
# Assemble a FineType bundle from FineType's own release assets.
#
#   scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR
#   scripts/fetch-finetype-bundle.sh --print-tag
#
# Input: the pinned tag (scripts/finetype-pin.sh, which reads
# packaging/finetype-pin.env) and a Rust target triple. Output: a directory of
# the shape scripts/check-bundled-extension.sh accepts and
# `brightfield_engine::semantic::bundle_beside` looks for —
#
#   finetype.duckdb_extension
#   model/…
#   taxonomy-schemas.json
#
# THE SOURCE IS THE RELEASE AND NOT A CHECKOUT. Not a sibling working tree, not
# the DuckDB community registry, not an expiring Actions artefact: a release
# asset is durable, checksummed, and addressed by the tag the pin already
# names, so the pin resolves to bytes without a second lookup and two builds of
# the same brightfield tag stage the same bytes.
#
# WHAT IS EXERCISED AND WHAT IS NOT — read this before trusting a green run.
# scripts/fetch-finetype-bundle-selftest.sh drives this whole file against a
# loopback HTTP server holding fixture assets, so the url construction, the
# fetch, the checksum comparison, the unpack and the assembly all run on every
# pull request. What no test here reaches is a REAL FineType release: at the
# time this was written a FineType tag published CLI tarballs and their
# checksums and nothing else, and the assets named below did not exist. The
# names are therefore an assumed shape — the same form as the CLI tarballs
# (`finetype-<tag>-<target>…` with a `.sha256` beside each) — and the first
# real release is where they are confirmed. A wrong name fails as a 404 during
# a release, loudly, which is the failure direction to want.
#
# BRIGHTFIELD_FINETYPE_ASSET_BASE replaces the release url with any base curl
# can fetch from. The self-test uses it; a release does not set it.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

TAG=$("${HERE}/finetype-pin.sh")

# The pin check runs this to compare what this script would use against the
# declaration and against what the other consumers answer. It is the same
# variable the fetch below uses, so the two cannot answer differently.
if [ "${1:-}" = "--print-tag" ]; then
  printf '%s\n' "$TAG"
  exit 0
fi

TARGET="${1:?usage: scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR}"
DEST="${2:?usage: scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR}"

BASE="${BRIGHTFIELD_FINETYPE_ASSET_BASE:-https://github.com/meridian-online/finetype/releases/download/${TAG}}"

fail() { echo "fetch-finetype-bundle: $*" >&2; exit 1; }

# The three assets, named once. A release that names them differently is one
# edit here and one in the self-test's fixture.
EXT_ASSET="finetype-${TAG}-${TARGET}.duckdb_extension"
CATALOGUE_ASSET="finetype-${TAG}-taxonomy-schemas.json"
MODEL_ASSET="finetype-${TAG}-model.tar.gz"

digest_of() {
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    sha256sum "$1" | awk '{print $1}'
  fi
}

WORK=$(mktemp -d "${TMPDIR:-/tmp}/bf-finetype-fetch.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

# get ASSET — fetch it and its .sha256 sidecar into $WORK, and refuse unless
# the bytes hash to what the sidecar says.
#
# THE VERIFICATION IS WHY THE DESTINATION IS BUILT LAST. Everything lands in
# $WORK, every digest is compared there, and only then is $DEST created — so a
# substituted or truncated download cannot reach the staged tree, cannot reach
# the manifest scripts/package.sh writes over it, and cannot be half-there for
# the next run to find and believe.
get() {
  local asset="$1"
  local url="${BASE}/${asset}"

  curl -sSfL --retry 3 --retry-delay 2 -o "${WORK}/${asset}" "$url" \
    || fail "could not fetch ${url}"
  curl -sSfL --retry 3 --retry-delay 2 -o "${WORK}/${asset}.sha256" "${url}.sha256" \
    || fail "could not fetch ${url}.sha256 — the asset published no checksum beside it"

  # `shasum -c` is deliberately not used: the sidecar names the file as the
  # publisher saw it, and the path here is this working directory's. The digest
  # is compared directly instead, which needs no agreement about paths.
  local declared actual
  declared=$(awk 'NR==1 {print $1}' "${WORK}/${asset}.sha256")
  printf '%s' "$declared" | grep -Eq '^[0-9a-f]{64}$' \
    || fail "${url}.sha256 did not parse to a sha256: '${declared}'"

  actual=$(digest_of "${WORK}/${asset}")
  [ "$declared" = "$actual" ] || fail "${url} does not match its published checksum —
  declared ${declared}
  actual   ${actual}
  the bytes are not the ones FineType released for ${TAG}"

  echo "   ${asset}  ${actual}"
}

echo "== fetch: FineType ${TAG} for ${TARGET}"
echo "   from ${BASE}"
get "$EXT_ASSET"
get "$CATALOGUE_ASSET"
get "$MODEL_ASSET"

# The model tarball's top-level shape is the one thing about these assets that
# a name cannot settle: an archive built with `tar -czf … model` carries a
# `model/` directory and one built with `-C model .` does not. Both are
# unpacked here and the required files are then looked for at one known place,
# so a third shape fails by name rather than producing a bundle missing its
# weights.
mkdir -p "${WORK}/unpacked"
tar -xzf "${WORK}/${MODEL_ASSET}" -C "${WORK}/unpacked" \
  || fail "${MODEL_ASSET} did not unpack"
MODEL_ROOT="${WORK}/unpacked"
if [ ! -f "${MODEL_ROOT}/model.safetensors" ]; then
  only=$(find "${WORK}/unpacked" -mindepth 1 -maxdepth 1 -type d)
  if [ "$(printf '%s\n' "$only" | grep -c .)" = "1" ] && [ -f "${only}/model.safetensors" ]; then
    MODEL_ROOT="$only"
  fi
fi
for required in model.safetensors config.json label_map.json \
                model2vec/model.safetensors model2vec/tokenizer.json; do
  [ -f "${MODEL_ROOT}/${required}" ] \
    || fail "${MODEL_ASSET} carries no ${required} — it is not the model directory a bundle needs"
done

# Assembled only now that every byte has been verified. `rm -rf` first so a
# destination left over from a previous run cannot contribute files nobody
# downloaded.
rm -rf "$DEST"
mkdir -p "$DEST"
cp "${WORK}/${EXT_ASSET}" "${DEST}/finetype.duckdb_extension"
cp "${WORK}/${CATALOGUE_ASSET}" "${DEST}/taxonomy-schemas.json"
# -L dereferences: a model archive holding links would otherwise produce a
# bundle that works only where those links resolve. check-bundled-extension.sh
# refuses a bundle with any symlink left in it, so this is the copy that has to
# not make one.
cp -RL "$MODEL_ROOT" "${DEST}/model"

# What model rode in that release, read out of the bytes rather than declared
# anywhere here. Printed, not asserted: the pin names a FineType tag and the
# tag decides the model, so a second declaration would be a pin nobody compares
# against anything.
model_name=$(sed -n 's/.*"value_embed_model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${DEST}/model/config.json" | head -1)
echo "== bundle: ${DEST}"
echo "   FineType ${TAG}, ${TARGET}, model ${model_name:-unnamed in config.json}"
