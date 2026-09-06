#!/usr/bin/env bash
# Refuse a PACKAGED ARTIFACT that carries no working type source.
#
#   scripts/check-artifact-type-source.sh ARTIFACT RUST_TARGET
#   scripts/check-artifact-type-source.sh --print-host-target
#
#   ARTIFACT      dist/brightfield-<version>-<target>.tar.gz, or the .dmg
#   RUST_TARGET   the triple that artifact was packaged for
#
# THERE IS NO FLAG FOR THE RUN, AND THAT IS THE POINT. Whether the packaged
# binary can be executed is decided here, from the target argument and the
# machine — it runs when they are the same triple and cannot when they are not.
# It used to be `--run`, supplied by the caller, and a review pass mutated the
# release workflow to pass nothing: every check in this repository stayed
# green while a release stopped ever loading the bundle it shipped. A caller
# cannot get wrong a decision it does not make.
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
#   3. THE EVIDENCE: the packaged binary's own `--check-type-source`. It loads
#      that extension with its own DuckDB, loads the model beside it and puts a
#      label on a column, reporting as an exit code. Steps 1 and 2 read a file
#      tree and a metadata trailer, and a bundle can satisfy both and still not
#      load — measured, on a bundle whose model.safetensors is 200 KB of random
#      bytes under the right name: all seven required files present,
#      check-bundled-extension.sh exit 0, and the packaged binary exit 1 with
#      the canary classifying three email addresses as unknown. Measured on
#      2026-09-06 against the installed FineType extension (ft_version reports
#      0.6.57; its metadata trailer says 0.6.23, which is a FineType defect
#      carded separately) and the real model at the pinned revision.
#      A file list is a pre-flight. This is the check.
#
#      It is skipped only where it CANNOT run: a cross-compiled artifact on a
#      runner of another architecture. The release matrix builds x86_64 on an
#      arm64 runner, so that leg is asset-verified and execution-unverified and
#      says so, which is the same position the workflow already records for the
#      Intel install path.
#
# THE PIN COMPARISON ASSUMES THE CHECKOUT BUILT THE ARTIFACT, which is true of
# its caller (release.yml, same job) and not of somebody running this over a
# tarball downloaded from an older release. Against one of those it reports the
# pin disagreeing, correctly and unhelpfully.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

# The triple this machine can execute — READ FROM TWO SOURCES THAT MUST AGREE,
# because this one value decides whether the bundle is ever loaded and a wrong
# answer turns the whole run leg off while every check stays green.
#
# `rustc -vV` is authoritative: it is the compiler that would build for this
# machine, it needs no table, and this is a Rust repository — scripts/package.sh
# already reads the same line. The uname table beside it exists to disagree. A
# single source cannot catch a typo in itself, and a self-test that asks the
# code under test what the host is cannot either: changing one arm of the table
# below used to leave every case in this repository green while a real release
# printed `== not run` for the artefact people download.
#
# An unknown host is a hard failure rather than a silent skip. So is a missing
# rustc, and the callers are why. This file is invoked by release.yml's build
# job, which installs the pinned toolchain to compile the binary it is checking,
# and by scripts/check-artifact-type-source-selftest.sh, which public-hygiene.yml
# and test.yml each now install that same pin for — a step they gained because
# of this line, rather than a property of the runner image they happened to
# have. So there is no caller for which a missing rustc means unavailable; it
# means a broken runner. The alternative, falling back to the uname table on its
# own, is the single source this cross-check exists to end.
host_target() {
  local from_rustc from_uname
  command -v rustc >/dev/null 2>&1 || {
    echo "check-artifact-type-source: rustc is not on PATH, so the host triple has only one" >&2
    echo "  source and nothing would catch it being wrong. Install the toolchain." >&2
    exit 1; }
  from_rustc=$(rustc -vV | sed -n 's/^host: //p')
  [ -n "$from_rustc" ] || {
    echo "check-artifact-type-source: rustc -vV printed no 'host:' line." >&2; exit 1; }

  case "$(uname -s)/$(uname -m)" in
    Darwin/arm64)   from_uname=aarch64-apple-darwin ;;
    Darwin/x86_64)  from_uname=x86_64-apple-darwin ;;
    Linux/aarch64)  from_uname=aarch64-unknown-linux-gnu ;;
    Linux/x86_64)   from_uname=x86_64-unknown-linux-gnu ;;
    *)
      echo "check-artifact-type-source: no Rust target triple known for $(uname -s)/$(uname -m)." >&2
      echo "  Add the mapping; a host nobody mapped must not silently skip the run." >&2
      exit 1 ;;
  esac

  [ "$from_rustc" = "$from_uname" ] || {
    echo "check-artifact-type-source: this machine's own two answers disagree — rustc says" >&2
    echo "  '${from_rustc}' and $(uname -s)/$(uname -m) maps to '${from_uname}'. One of them is" >&2
    echo "  wrong, and until it is fixed nothing here can decide whether an artefact is" >&2
    echo "  native, so nothing here may decide to skip loading it." >&2
    exit 1; }

  printf '%s\n' "$from_rustc"
}

# scripts/check-artifact-type-source-selftest.sh reads this so its fixtures are
# always for the machine running them, which is what keeps the run leg
# exercised on every runner rather than only on the one the fixture was written
# for.
if [ "${1:-}" = "--print-host-target" ]; then
  host_target
  exit 0
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

HOST=$(host_target)
if [ "$TARGET" = "$HOST" ]; then
  echo "== run: the packaged binary types a column"
  ts_status=0
  ( cd "$PKG" && "$EXE" --check-type-source ) > "$TMP/typesource.log" 2>&1 || ts_status=$?
  sed 's/^/   /' "$TMP/typesource.log"
  case "$ts_status" in
    0) echo "   ok: the packaged binary loaded the bundled extension and labelled a column" ;;
    2) fail "the binary reports no bundle beside it, but ${BUNDLE_REL} is in this artifact —
  it is staged somewhere the executable does not look. See scripts/package.sh." ;;
    *) fail "the packaged binary's type source did not come up (exit ${ts_status}).
  The bundle is present and well shaped and it does not LOAD. Read the message above:
  a missing directory the loader opens, an ABI the linked DuckDB refuses, or a model
  the canary could not classify. This is the failure a file check cannot see." ;;
  esac
else
  echo "== not run: this artifact is for ${TARGET} and this machine is ${HOST},"
  echo "   so the packaged binary cannot be executed here. The bundle was read"
  echo "   and NOT loaded; nothing below establishes that it would load."
fi

echo "check-artifact-type-source: ${ARTIFACT} carries FineType ${TAG} for ${PLATFORM}."
