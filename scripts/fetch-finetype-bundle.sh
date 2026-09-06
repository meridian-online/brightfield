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
#   The FineType release, addressed by the pinned tag, is to carry the
#   extension, the taxonomy catalogue and `finetype-model.json` — IT DOES NOT
#   YET, and the paragraph below says what that means. Each is to have a
#   `.sha256` published beside it, and each is refused unless the bytes hash to
#   it. That is an attestation: the checksum is written by the party that built
#   the artefact.
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
# WHICH ASSET NAMES ARE ASSUMED AND WHICH ARE MEASURED, stated precisely
# because the difference is the whole risk here. THE THREE RELEASE ASSETS DO
# NOT EXIST YET: `gh release view v0.6.58 --json assets` lists five CLI
# archives and their `.sha256` sidecars and nothing else. The names below are
# the ones a FineType change is adding, in the form those five already use, and
# the first release that carries them is where they are confirmed. A wrong name
# fails as a 404 during a release, loudly, which is the failure direction to
# want. The registry layout, by contrast, was read off the registry itself at
# the pinned revision. What no test reaches is the live release and the live
# registry together; the self-test drives this whole file against a loopback
# server standing in for both.
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
# THE MODEL. Named by the release, resolved at the pinned registry revision.
#
# THE FILE SET COMES FROM THE REGISTRY, NOT FROM A LIST IN THIS SCRIPT, and
# that is the correction of a defect rather than a preference. An earlier
# revision enumerated five filenames worked out from the model's own
# config.json. It missed `model2vec/`, which FineType's model loader opens
# unconditionally and which the config never mentions, and it missed two more
# files beside it — so the bundle assembled, satisfied every file check, and
# would not load. A longer hand-written list reproduces the same blindness one
# filename later.
#
# So both subtrees the revision publishes are mirrored whole:
#
#   <MODEL>/…      the classifier: weights, config, label map, and the optional
#                  second encoder when the model declares one
#   model2vec/…    at the REPOSITORY ROOT, not under the model — the encoder
#                  every model loads, published once and shared between them
#
# Nothing here decides which files those are. If the revision publishes another
# file under either prefix, it is fetched, verified and staged.
# ---------------------------------------------------------------------------
echo "== model: ${MODEL} at ${REVISION}"
echo "   from ${MODEL_ORIGIN}/${MODEL_REPO}"

tree_of() { # tree_of PREFIX OUT — the registry's listing for one subtree
  curl -sSfL --retry 3 --retry-delay 2 -o "$2" \
    "${MODEL_ORIGIN}/api/models/${MODEL_REPO}/tree/${REVISION}/$1?recursive=true" \
    || fail "the registry listed no files for $1 at ${REVISION} — either the revision does not
  carry it, or the pin's revision and the release's model name have drifted apart"
}

tree_of "$MODEL" "${WORK}/tree-model.json"
tree_of model2vec "${WORK}/tree-m2v.json"

# The plan: one line per file, `dest<TAB>remote<TAB>kind<TAB>digest`. Built from
# the two listings, so the download loop and the verification below cannot
# disagree about which files there are — they read the same plan.
#
# `model2vec/` from the root is staged at `model/model2vec/`, where the loader
# looks. A model publishing its own `model2vec/` inside its directory wins:
# that is the one the loader would open, and staging the shared copy over it
# would replace a deliberate choice with a default.
PLAN="${WORK}/plan.tsv"
python3 - "${WORK}/tree-model.json" "${WORK}/tree-m2v.json" "$MODEL" "$PLAN" <<'PLANPY'
import json, sys

model_tree, m2v_tree, model, out = sys.argv[1:5]


def files(path, strip):
    try:
        entries = json.load(open(path))
    except Exception as e:
        sys.exit(f"fetch-finetype-bundle: the registry listing did not parse: {e}")
    if not isinstance(entries, list):
        sys.exit("fetch-finetype-bundle: the registry listing is not a list of entries")
    found = []
    for e in entries:
        if e.get("type") != "file":
            continue
        remote = e.get("path", "")
        rel = remote[len(strip):] if strip and remote.startswith(strip) else remote
        lfs = e.get("lfs")
        if lfs and lfs.get("oid"):
            kind, digest = "sha256", lfs["oid"]
        elif e.get("oid"):
            kind, digest = "gitblob", e["oid"]
        else:
            sys.exit(f"fetch-finetype-bundle: the registry declares no digest for {remote}")
        found.append((rel, remote, kind, digest))
    return found


plan = {}
# The shared encoder first, so a model carrying its own overwrites it below.
for rel, remote, kind, digest in files(m2v_tree, ""):
    plan[rel] = (remote, kind, digest)
for rel, remote, kind, digest in files(model_tree, model + "/"):
    plan[rel] = (remote, kind, digest)

if not plan:
    sys.exit(f"fetch-finetype-bundle: the registry listed no files for {model} or model2vec")
for required in ("config.json", "label_map.json", "model.safetensors"):
    if required not in plan:
        sys.exit(f"fetch-finetype-bundle: the revision publishes no {model}/{required}")
if not any(r.startswith("model2vec/") for r in plan):
    sys.exit("fetch-finetype-bundle: the revision publishes no model2vec/ — FineType's loader "
             "opens that directory unconditionally, so a bundle without it cannot load")

with open(out, "w") as fh:
    for rel in sorted(plan):
        remote, kind, digest = plan[rel]
        fh.write(f"{rel}\t{remote}\t{kind}\t{digest}\n")
PLANPY

mkdir -p "${WORK}/model"
while IFS="$(printf '\t')" read -r rel remote _kind _digest; do
  mkdir -p "$(dirname "${WORK}/model/${rel}")"
  curl -sSfL --retry 3 --retry-delay 2 -o "${WORK}/model/${rel}" \
    "${MODEL_ORIGIN}/${MODEL_REPO}/resolve/${REVISION}/${remote}" \
    || fail "could not fetch ${remote} at ${REVISION}"
done < "$PLAN"

# Every planned file against the digest the registry declares for it. Read the
# header for what this establishes and what it does not.
#
# THERE IS NO PLAN-VERSUS-DISK SET COMPARISON, and its absence is deliberate.
# One was written and mutating it away left this self-test green: the download
# loop above writes exactly the plan into a fresh temp directory or calls
# `fail`, so nothing can reach the disk that the plan does not name, or fail to
# reach it. A comparison that cannot fail reads exactly like one that works.
# The guard that CAN fail is the empty-plan refusal in the plan builder above,
# and the self-test drives it by emptying both registry listings.
python3 - "$PLAN" "${WORK}/model" <<'VERIFYPY' || exit 1
import hashlib, os, sys

plan_path, root = sys.argv[1:3]
plan = {}
for line in open(plan_path):
    rel, _remote, kind, digest = line.rstrip("\n").split("\t")
    plan[rel] = (kind, digest)

for rel, (kind, want) in sorted(plan.items()):
    data = open(os.path.join(root, rel), "rb").read()
    got = (hashlib.sha256(data).hexdigest() if kind == "sha256"
           else hashlib.sha1(b"blob %d\0" % len(data) + data).hexdigest())
    if got != want:
        sys.exit(f"fetch-finetype-bundle: {rel} does not match the registry's {kind} for it "
                 f"at this revision —\n  declared {want}\n  actual   {got}")

print(f"   {len(plan)} model files match the registry's digests at this revision")
VERIFYPY

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
echo "   FineType ${TAG}, ${TARGET}, model ${MODEL} at ${REVISION}"
echo "   the bundle is assembled, which is not the same as loadable — that is"
echo "   scripts/check-artifact-type-source.sh's question, and it runs the binary"
