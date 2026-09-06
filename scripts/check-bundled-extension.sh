#!/usr/bin/env bash
# Refuse a FineType bundle that would not load.
#
#   scripts/check-bundled-extension.sh BUNDLE_DIR [EXPECTED_PLATFORM] [EXPECTED_VERSION]
#
# A bundle is three things and this reads all of them:
#
#   finetype.duckdb_extension   the DuckDB loadable extension
#   model/                      what FINETYPE_MODEL_DIR is pointed at, including
#                               the value-embedding directory model/config.json
#                               names in `value_embed_model`
#   taxonomy-schemas.json       one JSON Schema per label, for value checking
#
# Two callers, one reading. scripts/package.sh runs it over the bundle it is
# about to stage, with the DuckDB platform name for the target being packaged;
# scripts/verify-airgapped.sh runs it over the bundle it found INSIDE a built
# artifact, with no platform argument (the artifact does not know which target
# it is, and package.sh already decided). Neither re-implements the trailer
# offsets, which is the point of the file.
#
# WHAT EACH CHECK IS FOR — none of these is a formality:
#
#   The metadata trailer. DuckDB will not LOAD a shared library that was never
#   stamped, and the failure surfaces at runtime as an extension that simply
#   is not there.
#
#   C_STRUCT. That is DuckDB's stable C API, and it is the only reason one
#   extension artifact survives a DuckDB patch bump at all. An extension built
#   against the unstable API is pinned to one exact DuckDB release; bundling
#   one into an application whose own DuckDB moves independently produces a
#   package that breaks on the next dependency bump, silently, for everyone.
#
#   The platform. Cross-packaging is the failure mode: a packaging run on an
#   arm64 machine that produces the x86_64 artifact with an arm64 extension in
#   it yields a tarball that fails on every machine it was built FOR and works
#   on the one machine it will never be tested on.
#
#   The version, when the caller declares one. scripts/package.sh passes the
#   tag from packaging/finetype-pin.env, so the extension's own version stamp
#   — written by FineType's build, not by anything in this repository — is
#   compared against the pin. Without that the pin is a string two scripts
#   agree about, and a bundle assembled from some other release stages happily
#   under a declaration that says otherwise. scripts/verify-airgapped.sh
#   passes no version: it reads a bundle inside a built artifact and the
#   packaging run already made that comparison against the pin of the commit
#   that built it, which is not necessarily the pin of the checkout reading it.
#
#   Symlinks. A model fetched through a content-addressed cache is a tree of
#   links into that cache. `cp -R` copies the links; the artifact then works
#   perfectly on the packaging machine and nowhere else, so no amount of
#   testing where it was built will show it.
#
#   The manifest, when the bundle carries one. scripts/package.sh records a
#   hash per staged file; a mismatch means the artifact is not carrying the
#   bytes that were packaged. It is not a signature — it sits in the directory
#   it describes — and a bundle nobody has packaged has none, so the check is
#   conditional on its presence and strict once it is there.
#
# WHAT IT DOES NOT CHECK, AND SHOULD: the extension's own load commands. This
# reads the metadata trailer and the file tree; it never asks what the shared
# library needs in order to load. An extension linking a non-system dylib — a
# vendored OpenSSL, a Homebrew libomp — would pass every check here and fail to
# open on a machine that does not have it, which is the same shape of failure as
# the symlink case above and is caught by none of the same checks.
#
# Measured on the extension this branch bundles, so the gap is recorded rather
# than asserted: its five LC_LOAD_DYLIB entries are all system paths
# (Security, SystemConfiguration, CoreFoundation, libiconv, libSystem), so that
# artifact is portable. What it does carry is an LC_ID_DYLIB of
# `<builder>/finetype/target/release/deps/libfinetype.dylib` — the file's own
# install name, not a dependency it loads, so it does not stop the extension
# opening anywhere. It does put an absolute path from the build machine inside a
# public release artifact.
#
# Adding the check is a separate piece of work: it needs an OS allowlist per
# target, which scripts/package.sh already maintains for the executable and
# which would have to be shared rather than duplicated.
#
# WHAT IT DOES NOT CHECK: that the model is a model. A file of the right name
# and the wrong bytes passes here (unless a manifest contradicts it) and is
# caught at runtime by the canary in
# `brightfield_engine::semantic::FinetypeBundle::open`, which makes the loaded
# extension classify three email addresses before the bundle is accepted. It
# also does not compare the trailer's DuckDB version against the DuckDB this
# workspace links — only the linked library knows its own version, so that
# comparison lives in `semantic::check_abi`.
set -euo pipefail

BUNDLE="${1:?usage: scripts/check-bundled-extension.sh BUNDLE_DIR [EXPECTED_PLATFORM] [EXPECTED_VERSION]}"
WANT_PLATFORM="${2:-}"
WANT_VERSION="${3:-}"

fail() { echo "check-bundled-extension: $*" >&2; exit 1; }

[ -d "$BUNDLE" ] || fail "no such directory: ${BUNDLE}"

EXT="${BUNDLE}/finetype.duckdb_extension"
for required in \
  finetype.duckdb_extension \
  model/model.safetensors \
  model/config.json \
  model/label_map.json \
  taxonomy-schemas.json
do
  [ -f "${BUNDLE}/${required}" ] || fail "${BUNDLE} carries no ${required}"
done

# THE VALUE-EMBEDDING MODEL IS NAMED BY THE MODEL'S OWN CONFIG, not by this
# file. `model/config.json` carries `value_embed_model`, a directory name
# relative to the model directory, and that is what the extension opens.
#
# This used to require `model/model2vec/…` as a literal. The published model
# names its directory `value_model2vec`, so the literal was a requirement no
# real bundle could satisfy and every bundle that did satisfy it was one
# somebody had built by hand to match the check. Reading the config asks the
# question the extension asks.
embed=$(sed -n 's/.*"value_embed_model"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
  "${BUNDLE}/model/config.json" | head -1)
[ -n "$embed" ] || fail "${BUNDLE}/model/config.json declares no value_embed_model, so the \
extension has no name for the directory it embeds values with"
case "$embed" in
  */*|..|.) fail "${BUNDLE}/model/config.json declares value_embed_model '${embed}', which is \
a path rather than a directory name beside the model" ;;
esac
for required in model.safetensors tokenizer.json; do
  [ -f "${BUNDLE}/model/${embed}/${required}" ] \
    || fail "${BUNDLE} carries no model/${embed}/${required}, which model/config.json names \
through value_embed_model"
done

# Self-containedness. -type l finds symlinks whether or not they resolve, so a
# link that happens to work on this machine still fails.
strays=$(find "$BUNDLE" -type l | wc -l | tr -d ' ')
if [ "$strays" -ne 0 ]; then
  find "$BUNDLE" -type l | sed 's/^/  /' >&2
  fail "${strays} symlink(s) inside the bundle — it is not self-contained (use cp -RL)"
fi

# The DuckDB metadata trailer: the last 512 bytes are eight 32-byte NUL-padded
# ASCII fields written LAST-FIRST, then 256 bytes of signature space. Field 1
# (the magic, "4") therefore lands at offset 224, and below it sit the platform
# (192), the DuckDB version (160), the extension version (128) and the ABI type
# (96). `brightfield_engine::semantic::read_stamp` reads the same offsets.
field() { tail -c 512 "$EXT" | dd bs=1 skip="$1" count=32 2>/dev/null | tr -d '\0'; }

size=$(wc -c < "$EXT" | tr -d ' ')
[ "$size" -ge 512 ] || fail "${EXT} is ${size} bytes — shorter than the metadata trailer"

magic=$(field 224)
[ "$magic" = "4" ] || fail "${EXT} carries no DuckDB metadata trailer (magic '${magic}'); \
an unstamped shared library will not LOAD"

abi=$(field 96)
[ "$abi" = "C_STRUCT" ] || fail "${EXT} declares ABI '${abi}', not C_STRUCT — only DuckDB's \
stable C API survives a DuckDB version bump"

platform=$(field 192)
version=$(field 128)
duckdb_floor=$(field 160)

# The manifest scripts/package.sh writes over the staged copy. Absent for a
# locally assembled bundle nobody has packaged, which is why this is
# conditional; present and disagreeing means the artifact is not carrying the
# bytes that were packaged, which no other check here can see. The application
# verifies the same file before it loads the extension.
manifest="${BUNDLE}/bundle-manifest.sha256"
manifest_note="no manifest"
if [ -f "$manifest" ]; then
  lines=$(grep -c . "$manifest" || true)
  [ "${lines:-0}" -gt 0 ] || fail "${manifest} names no files, so it verifies nothing"
  ( cd "$BUNDLE" && shasum -a 256 -c --status bundle-manifest.sha256 ) \
    || fail "${BUNDLE} does not match its own manifest — the bundle is not the one that \
was packaged (run 'shasum -a 256 -c bundle-manifest.sha256' in it to see which file)"
  manifest_note="${lines} files match the manifest"
fi

if [ -n "$WANT_PLATFORM" ] && [ "$platform" != "$WANT_PLATFORM" ]; then
  fail "${EXT} is built for '${platform}' and this artifact needs '${WANT_PLATFORM}' — \
DuckDB would refuse to load it on every machine the artifact is for"
fi

# Compared with a leading `v` stripped from both sides, because a git tag
# carries one and a DuckDB extension version stamp does not have to. Nothing
# else is normalised: a stamp of `0.6.57` under a pin of `v0.6.58` is the
# defect this exists to catch, and a tolerant comparison would let it through.
if [ -n "$WANT_VERSION" ]; then
  want_core="${WANT_VERSION#v}"
  have_core="${version#v}"
  [ "$want_core" = "$have_core" ] || fail "${EXT} is stamped version '${version}' and the pin in \
packaging/finetype-pin.env declares '${WANT_VERSION}' — this bundle is not the FineType release \
this repository says it stages"
fi

echo "check-bundled-extension: finetype ${version}, ${abi}, ${platform}, DuckDB floor ${duckdb_floor}, no symlinks, ${manifest_note}."
