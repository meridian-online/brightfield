#!/usr/bin/env bash
# Assemble a FineType bundle from the FineType release and the model registry.
#
#   scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR
#   scripts/fetch-finetype-bundle.sh --print-tag
#
# Input: the pin (scripts/finetype-pin.sh, reading packaging/finetype-pin.env)
# and a Rust target triple. Output: a directory of the shape
# scripts/check-bundled-extension.sh accepts and
# `brightfield_engine::semantic::bundle_beside` looks for —
#
#   finetype.duckdb_extension
#   model/…                     including the directory model/config.json names
#                               in `value_embed_model`
#   taxonomy-schemas.json
#
# TWO SOURCES, AND THE ASYMMETRY BETWEEN THEM IS THE THING TO UNDERSTAND.
#
#   The FineType release, addressed by the pinned tag, attaches the extension,
#   the taxonomy catalogue and `finetype-model.json`. Each has a `.sha256`
#   published beside it by the publisher, and each is refused unless the bytes
#   hash to it. That is an attestation: the checksum was written by the party
#   that built the artefact.
#
#   The model registry, addressed by the pinned revision, holds the weights.
#   The FineType tag does not attach them and publishes no checksum for them,
#   so there is nothing here to compare against a second party. What IS done:
#   the registry's own file listing at the pinned revision is fetched once and
#   every downloaded file is checked against the digest it carries there —
#   sha256 for the large files it stores as LFS objects, the git blob id for
#   the small ones. That catches a truncated or corrupted transfer and it
#   catches a file that is not the one the revision names. It does NOT
#   independently attest the bytes: the listing and the files come from the
#   same server, so a registry serving different content would be self-
#   consistent. Do not read a green run here as the model being checksummed
#   the way the release assets are.
#
# WHICH ASSET NAMES ARE ASSUMED AND WHICH ARE MEASURED. The three release asset
# names are the ones FineType's release now publishes. The registry layout was
# read off the registry itself at the pinned revision. What no test reaches is
# the live release and the live registry together; the self-test drives this
# whole file against a loopback server standing in for both.
#
# BRIGHTFIELD_FINETYPE_ASSET_BASE replaces the release url and
# BRIGHTFIELD_FINETYPE_MODEL_ORIGIN the registry's. The self-test sets both; a
# release sets neither.
#
# Needs python3 for JSON. The release runner has it and so does every developer
# machine here; it is a hard failure rather than a fallback, because the
# alternative is parsing a registry listing with sed.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

TAG=$("${HERE}/finetype-pin.sh" --tag)

# The pin check runs this to compare what this script would use against the
# declaration and against what the other consumers answer. It is the same
# variable the fetch below uses, so the two cannot answer differently.
if [ "${1:-}" = "--print-tag" ]; then
  printf '%s\n' "$TAG"
  exit 0
fi

TARGET="${1:?usage: scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR}"
DEST="${2:?usage: scripts/fetch-finetype-bundle.sh RUST_TARGET DEST_DIR}"

REVISION=$("${HERE}/finetype-pin.sh" --revision)

RELEASE_BASE="${BRIGHTFIELD_FINETYPE_ASSET_BASE:-https://github.com/meridian-online/finetype/releases/download/${TAG}}"
MODEL_ORIGIN="${BRIGHTFIELD_FINETYPE_MODEL_ORIGIN:-https://huggingface.co}"
MODEL_REPO="meridian-online/finetype-model"

fail() { echo "fetch-finetype-bundle: $*" >&2; exit 1; }

command -v python3 >/dev/null 2>&1 || fail "python3 is required to read the release metadata"

# The destination is cleared HERE, before a single byte is fetched, so a run
# that fails for any reason leaves nothing behind. The alternative — clearing
# it at assembly time — leaves a bundle from an earlier run sitting where the
# next scripts/package.sh invocation would find it and stage it, which is
# precisely the silent substitution the checksums below exist to prevent.
case "$DEST" in
  ""|"/") fail "refusing to use '${DEST}' as a bundle directory" ;;
esac
[ ! -e "$DEST" ] || [ -d "$DEST" ] || fail "${DEST} exists and is not a directory"
rm -rf "$DEST"

# The three release assets, named once. Only the extension is per-target.
EXT_ASSET="finetype-${TAG}-${TARGET}.duckdb_extension"
CATALOGUE_ASSET="taxonomy-schemas.json"
METADATA_ASSET="finetype-model.json"

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
# substituted or truncated download cannot reach the staged tree and cannot
# reach the manifest scripts/package.sh writes over it.
get() {
  local asset="$1"
  local url="${RELEASE_BASE}/${asset}"

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

echo "== release: FineType ${TAG} for ${TARGET}"
echo "   from ${RELEASE_BASE}"
get "$EXT_ASSET"
get "$CATALOGUE_ASSET"
get "$METADATA_ASSET"

# The release's own record of what it is. Its `tag` field is compared against
# the pin, which is a cross-check the extension's trailer cannot give: it says
# the RELEASE we fetched from is the one the pin names, independently of what
# any single asset was stamped with.
read -r META_TAG MODEL <<EOF
$(python3 - "${WORK}/${METADATA_ASSET}" <<'PY'
import json, sys
try:
    meta = json.load(open(sys.argv[1]))
except Exception as e:
    sys.exit(f"finetype-model.json did not parse: {e}")
tag, model = meta.get("tag"), meta.get("model")
if not tag or not model:
    sys.exit("finetype-model.json carries no 'tag' and 'model' pair")
if "/" in model or model in ("", ".", ".."):
    sys.exit(f"finetype-model.json names model {model!r}, which is not a directory name")
print(tag, model)
PY
)
EOF
[ -n "${MODEL:-}" ] || fail "could not read the model name from ${METADATA_ASSET}"
[ "$META_TAG" = "$TAG" ] || fail "${METADATA_ASSET} says it was built for '${META_TAG}' and the \
pin declares '${TAG}' — this is not the release packaging/finetype-pin.env names"

# ---------------------------------------------------------------------------
# The model. Named by the release, resolved at the pinned registry revision.
# ---------------------------------------------------------------------------
echo "== model: ${MODEL} at ${REVISION}"
echo "   from ${MODEL_ORIGIN}/${MODEL_REPO}"

TREE="${WORK}/tree.json"
curl -sSfL --retry 3 --retry-delay 2 -o "$TREE" \
  "${MODEL_ORIGIN}/api/models/${MODEL_REPO}/tree/${REVISION}/${MODEL}?recursive=true" \
  || fail "the registry listed no files for ${MODEL} at ${REVISION} — either the revision does \
not carry that model, or the pin's revision and the release's model name have drifted apart"

mkdir -p "${WORK}/model"
model_get() { # model_get RELATIVE_PATH
  local rel="$1"
  mkdir -p "$(dirname "${WORK}/model/${rel}")"
  curl -sSfL --retry 3 --retry-delay 2 -o "${WORK}/model/${rel}" \
    "${MODEL_ORIGIN}/${MODEL_REPO}/resolve/${REVISION}/${MODEL}/${rel}" \
    || fail "could not fetch ${MODEL}/${rel} at ${REVISION}"
}

model_get config.json
model_get label_map.json
model_get model.safetensors

# The value-embedding directory is named by the model's own config, not by this
# script. scripts/check-bundled-extension.sh reads the same field, so the file
# that decides which directory is fetched is the file that decides which one is
# required.
EMBED=$(sed -n 's/.*"value_embed_model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${WORK}/model/config.json" | head -1)
[ -n "$EMBED" ] || fail "${MODEL}/config.json declares no value_embed_model, so there is no name \
for the directory the extension embeds values with"
case "$EMBED" in
  */*|..|.) fail "${MODEL}/config.json declares value_embed_model '${EMBED}', which is a path \
rather than a directory name beside the model" ;;
esac
model_get "${EMBED}/model.safetensors"
model_get "${EMBED}/tokenizer.json"

# Every downloaded file against the digest the registry's listing carries for
# it at this revision. Read the header above for exactly what this establishes
# and what it does not.
python3 - "$TREE" "${WORK}/model" "$MODEL" <<'PY' || exit 1
import hashlib, json, os, sys

tree_path, root, model = sys.argv[1:4]
try:
    entries = json.load(open(tree_path))
except Exception as e:
    sys.exit(f"fetch-finetype-bundle: the registry listing did not parse: {e}")
if not isinstance(entries, list) or not entries:
    sys.exit(f"fetch-finetype-bundle: the registry listed no files for {model}")

prefix = model + "/"
declared = {}
for entry in entries:
    if entry.get("type") != "file":
        continue
    path = entry.get("path", "")
    rel = path[len(prefix):] if path.startswith(prefix) else path
    lfs = entry.get("lfs")
    if lfs and lfs.get("oid"):
        declared[rel] = ("sha256", lfs["oid"])
    elif entry.get("oid"):
        declared[rel] = ("gitblob", entry["oid"])

checked = 0
for dirpath, _, names in os.walk(root):
    for name in names:
        full = os.path.join(dirpath, name)
        rel = os.path.relpath(full, root)
        if rel not in declared:
            sys.exit(f"fetch-finetype-bundle: {model}/{rel} is not in the registry's listing "
                     f"for revision — it is not a file this revision publishes")
        kind, want = declared[rel]
        data = open(full, "rb").read()
        if kind == "sha256":
            got = hashlib.sha256(data).hexdigest()
        else:
            got = hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest()
        if got != want:
            sys.exit(f"fetch-finetype-bundle: {model}/{rel} does not match the registry's "
                     f"{kind} for it at this revision —\n  declared {want}\n  actual   {got}")
        checked += 1

# Reading zero files is a failure, not a pass. An empty download directory
# would otherwise satisfy every comparison above by making none of them.
if checked == 0:
    sys.exit("fetch-finetype-bundle: no model files were downloaded, so nothing was verified")
print(f"   {checked} model files match the registry's digests at this revision")
PY

# Assembled only now that every byte has been verified.
mkdir -p "$DEST"
cp "${WORK}/${EXT_ASSET}" "${DEST}/finetype.duckdb_extension"
cp "${WORK}/${CATALOGUE_ASSET}" "${DEST}/taxonomy-schemas.json"
# -L dereferences: an archive holding links would otherwise produce a bundle
# that works only where those links resolve. check-bundled-extension.sh refuses
# a bundle with any symlink left in it, so this is the copy that has to not
# make one.
cp -RL "${WORK}/model" "${DEST}/model"

echo "== bundle: ${DEST}"
echo "   FineType ${TAG}, ${TARGET}, model ${MODEL} (${EMBED}) at ${REVISION}"
