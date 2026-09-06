#!/usr/bin/env bash
# Refuse a PACKAGED ARTIFACT that carries no working type source.
#
#   scripts/check-artifact-type-source.sh [--run] ARTIFACT RUST_TARGET
#
#   ARTIFACT      dist/brightfield-<version>-<target>.tar.gz, or the .dmg
#   RUST_TARGET   the triple that artifact was packaged for
#   --run         also execute the packaged binary's --check-type-source
#
# WHY THIS IS NOT scripts/check-bundled-extension.sh, AND WHY IT IS NOT
# scripts/verify-airgapped.sh. check-bundled-extension.sh reads a directory
# somebody hands it — on a release that is the STAGING INPUT, before packaging,
# so it says nothing about what came out. verify-airgapped.sh does read the
# artifact, and it SKIPS the type-source legs when the artifact carries no
# bundle, because a build without one is supported; it also reports without
# gating the release. Neither of them can fail on the case that actually
# shipped: a release whose packaging step quietly staged nothing.
#
# That case is not hypothetical. `stage_finetype` in scripts/package.sh opens
# `[ -n "$FINETYPE_BUNDLE" ] || return 0`, and for as long as no workflow set
# BRIGHTFIELD_FINETYPE_BUNDLE every artifact the release built was staged with
# no bundle — while a card recorded the extension and its model as shipping,
# verified three times by reviewers who each assembled a bundle by hand. The
# mechanism was real and the fixtures were real; the path a release takes was
# what nobody exercised. So this reads the unpacked artifact, and ABSENCE IS A
# FAILURE.
#
# What it checks, in order:
#
#   1. The bundle is at the path `brightfield_engine::semantic::bundle_beside`
#      looks in — `finetype/` beside the executable in the tarball,
#      `Contents/Resources/finetype` in the app bundle — and not merely
#      somewhere inside the artifact. A bundle staged one directory across is
#      an artifact the binary will never find.
#   2. scripts/check-bundled-extension.sh over it, with the DuckDB platform for
#      RUST_TARGET and the tag from packaging/finetype-pin.env. That is the
#      cross-packaging check on the artifact rather than on the input, which is
#      the one that matters for a cross-compiled leg the runner cannot execute,
#      and it verifies the manifest scripts/package.sh wrote over the staged
#      copy.
#   3. With --run, the packaged binary's own `--check-type-source`: it loads
#      that extension with its own DuckDB, loads the model beside it and puts a
#      label on a column, reporting as an exit code. Only for a native leg — a
#      cross-compiled artifact cannot be executed on the runner that built it.
#
# THE PIN COMPARISON ASSUMES THE CHECKOUT BUILT THE ARTIFACT, which is true of
# its caller (release.yml, same job) and not of somebody running this over a
# tarball downloaded from an older release. Against one of those it reports the
# pin disagreeing, correctly and unhelpfully.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

RUN=0
if [ "${1:-}" = "--run" ]; then
  RUN=1
  shift
fi

ARTIFACT="${1:?usage: scripts/check-artifact-type-source.sh [--run] ARTIFACT RUST_TARGET}"
TARGET="${2:?usage: scripts/check-artifact-type-source.sh [--run] ARTIFACT RUST_TARGET}"

fail() { echo "check-artifact-type-source: $*" >&2; exit 1; }

[ -f "$ARTIFACT" ] || fail "no such artifact: ${ARTIFACT}"

PLATFORM=$("${HERE}/duckdb-platform.sh" "$TARGET")
TAG=$("${HERE}/finetype-pin.sh")

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-artifact-typesource.XXXXXX")
MOUNT=""
# Detach before the temp tree goes: the mount point lives inside it, and
# removing a directory an image is mounted on leaves the image attached.
#
# THE RETRY IS NOT DEFENSIVE PADDING. A plain detach immediately after running
# a binary out of the image fails with "resource busy" often enough to matter —
# the kernel has not finished releasing the executable's vnode — and the
# `rm -rf` then hits a read-only mount, fails, and takes the script's exit
# status with it. That reported a PASSING check as a failure. So: a few
# attempts, then force, and cleanup can never decide the exit status.
cleanup() {
  local rc=$?
  if [ -n "$MOUNT" ]; then
    local n=0
    until hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1; do
      n=$((n + 1))
      [ "$n" -lt 5 ] || { hdiutil detach "$MOUNT" -force -quiet >/dev/null 2>&1 || true; break; }
      sleep 1
    done
  fi
  rm -rf "$TMP" 2>/dev/null || true
  return "$rc"
}
trap cleanup EXIT

case "$ARTIFACT" in
  *.tar.gz)
    echo "== unpack: ${ARTIFACT}"
    tar -xzf "$ARTIFACT" -C "$TMP"
    PKG=$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)
    [ -n "$PKG" ] || fail "${ARTIFACT} unpacked to no directory"
    BUNDLE_REL="finetype"
    EXE="./brightfield"
    ;;
  *.dmg)
    [ "$(uname -s)" = "Darwin" ] || fail "a .dmg can only be attached on macOS"
    mkdir -p "$TMP/volume"
    echo "== attach: ${ARTIFACT}"
    hdiutil attach -nobrowse -noverify -readonly -mountpoint "$TMP/volume" "$ARTIFACT"
    MOUNT="$TMP/volume"
    PKG="$MOUNT"
    BUNDLE_REL="Brightfield.app/Contents/Resources/finetype"
    EXE="./Brightfield.app/Contents/MacOS/brightfield"
    ;;
  *)
    fail "not an artifact this script knows: ${ARTIFACT} (expected .tar.gz or .dmg)"
    ;;
esac

[ -x "$PKG/$EXE" ] || fail "${ARTIFACT} carries no executable at ${EXE}"

# The whole point of the file. Named at the path the binary looks in, so a
# bundle that landed anywhere else reads as absent — which is what it is.
[ -d "$PKG/$BUNDLE_REL" ] || fail "${ARTIFACT} carries no type source at ${BUNDLE_REL}.
  brightfield_engine::semantic::bundle_beside looks there and nowhere else, so this
  artifact would report every column's storage type and no semantic label.
  scripts/package.sh stages it only when BRIGHTFIELD_FINETYPE_BUNDLE is set."

echo "== type source: ${BUNDLE_REL}"
"${HERE}/check-bundled-extension.sh" "$PKG/$BUNDLE_REL" "$PLATFORM" "$TAG" | sed 's/^/   /'

if [ "$RUN" -eq 1 ]; then
  echo "== run: the packaged binary types a column"
  ts_status=0
  ( cd "$PKG" && "$EXE" --check-type-source ) > "$TMP/typesource.log" 2>&1 || ts_status=$?
  sed 's/^/   /' "$TMP/typesource.log"
  case "$ts_status" in
    0) echo "   ok: the packaged binary loaded the bundled extension and labelled a column" ;;
    2) fail "the binary reports no bundle beside it, but ${BUNDLE_REL} is in this artifact —
  it is staged somewhere the executable does not look. See scripts/package.sh." ;;
    *) fail "the packaged binary's type source did not come up (exit ${ts_status})" ;;
  esac
else
  echo "== not run: --run was not given, so the extension was read and not loaded"
fi

echo "check-artifact-type-source: ${ARTIFACT} carries FineType ${TAG} for ${PLATFORM}."
