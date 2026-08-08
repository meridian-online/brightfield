#!/usr/bin/env bash
# Refuse a FineType bundle that would not load.
#
#   scripts/check-bundled-extension.sh BUNDLE_DIR [EXPECTED_PLATFORM]
#
# A bundle is three things and this reads all of them:
#
#   finetype.duckdb_extension   the DuckDB loadable extension
#   model/                      what FINETYPE_MODEL_DIR is pointed at
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
# WHAT IT DOES NOT CHECK: that the model is a model. A file of the right name
# and the wrong bytes passes here (unless a manifest contradicts it) and is
# caught at runtime by the canary in
# `brightfield_engine::semantic::FinetypeBundle::open`, which makes the loaded
# extension classify three email addresses before the bundle is accepted. It
# also does not compare the trailer's DuckDB version against the DuckDB this
# workspace links — only the linked library knows its own version, so that
# comparison lives in `semantic::check_abi`.
set -euo pipefail

BUNDLE="${1:?usage: scripts/check-bundled-extension.sh BUNDLE_DIR [EXPECTED_PLATFORM]}"
WANT_PLATFORM="${2:-}"

fail() { echo "check-bundled-extension: $*" >&2; exit 1; }

[ -d "$BUNDLE" ] || fail "no such directory: ${BUNDLE}"

EXT="${BUNDLE}/finetype.duckdb_extension"
for required in \
  finetype.duckdb_extension \
  model/model.safetensors \
  model/config.json \
  model/label_map.json \
  model/model2vec/model.safetensors \
  model/model2vec/tokenizer.json \
  taxonomy-schemas.json
do
  [ -f "${BUNDLE}/${required}" ] || fail "${BUNDLE} carries no ${required}"
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

echo "check-bundled-extension: finetype ${version}, ${abi}, ${platform}, DuckDB floor ${duckdb_floor}, no symlinks, ${manifest_note}."
