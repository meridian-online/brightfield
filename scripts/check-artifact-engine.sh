#!/usr/bin/env bash
# Refuse a PACKAGED ARTIFACT that carries no working DuckDB engine.
#
#   scripts/check-artifact-engine.sh ARTIFACT RUST_TARGET
#
#   ARTIFACT      dist/brightfield-<version>-<target>.tar.gz, or the .dmg
#   RUST_TARGET   the triple that artifact was packaged for
#
# WHY IT EXISTS. The Run control hands `arc` the DuckDB CLI staged beside the
# executable, and an app started from Finder has no other duckdb to fall back
# on. scripts/package.sh stages it only when BRIGHTFIELD_DUCKDB_CLI is set, and
# with the variable unset it prints `== engine: none` and exits 0 — a supported
# local build, and a release that ships a Run control which cannot run. Nothing
# in the release read the engine back before this, so deleting that one line
# from release.yml published green. So this reads the unpacked artifact, and
# ABSENCE IS A FAILURE.
#
# What it checks, in order:
#
#   1. The engine is at the path `staged_engine_beside` in
#      crates/brightfield-shell/src/run.rs looks in — `engine/duckdb` beside the
#      executable in the tarball, `Contents/Helpers/duckdb` in the app — and is
#      executable.
#   2. Its bytes are the pinned CLI for RUST_TARGET (scripts/fetch-duckdb-cli.sh
#      --check), and the version that pin names is the one `libduckdb-sys` in
#      Cargo.lock builds into the binary. The release builds with `--locked`,
#      so the lockfile is what was linked. These two hold on every leg,
#      including one this machine cannot execute.
#   3. THE EVIDENCE, on the leg that can run it: the packaged binary's own
#      `--check-engine`. It finds the engine the way the Run control does, from
#      its own path, runs a query with it in an empty environment, and compares
#      the DuckDB version and commit it reports with the DuckDB the binary
#      links. Exit 2 from it means the engine is in the artifact somewhere the
#      binary does not look.
#
#      Skipped only where it CANNOT run, as scripts/check-artifact-type-source.sh
#      skips its load: a cross-compiled artifact on a runner of another
#      architecture. That decision is made here from the target and the
#      machine, and recorded in the run summary and as a workflow warning.
#
# The host triple comes from scripts/check-artifact-type-source.sh
# --print-host-target, which reads it from two sources that must agree and is
# self-tested for it; a second copy here would be free to disagree.
set -euo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)

ARTIFACT="${1:?usage: scripts/check-artifact-engine.sh ARTIFACT RUST_TARGET}"
TARGET="${2:?usage: scripts/check-artifact-engine.sh ARTIFACT RUST_TARGET}"

fail() { echo "check-artifact-engine: $*" >&2; exit 1; }

[ -f "$ARTIFACT" ] || fail "no such artifact: ${ARTIFACT}"

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-artifact-engine.XXXXXX")
MOUNT=""
# Set only after the last check, for the reason scripts/check-artifact-type-source.sh
# gives at length: a `set -u` abort reaches the EXIT trap with `$?` at 0, and
# this check reporting 0 is a release publishing an engine nothing read.
COMPLETED=""
# Detach before the temp tree goes, retried: a detach right after running a
# binary out of the image fails "resource busy" often enough to matter.
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
  if [ -z "$COMPLETED" ] && [ "$rc" -eq 0 ]; then
    echo "check-artifact-engine: stopped before reaching the end and reported no reason." >&2
    echo "  Exiting 1: a check that did not finish has established nothing." >&2
    return 1
  fi
  return "$rc"
}
trap cleanup EXIT

# attach_image DMG MOUNTPOINT — hdiutil attach, retried, detaching whatever a
# failed attempt left behind; the same measured flake and the same answer as
# attach_image in scripts/check-artifact-type-source.sh.
attach_image() {
  local n=0 dev img
  img="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
  until hdiutil attach -nobrowse -noverify -readonly -mountpoint "$2" "$1" >/dev/null; do
    for dev in $(hdiutil info | awk -v p="$img" '
      $1 == "image-path" { hit = (index($0, p) > 0); next }
      hit && $1 ~ /^\/dev\/disk[0-9]+$/ { print $1; hit = 0 }'); do
      hdiutil detach "$dev" -force -quiet >/dev/null 2>&1 || true
    done
    n=$((n + 1))
    [ "$n" -lt 4 ] || return 1
    sleep 2
  done
}

case "$ARTIFACT" in
  *.tar.gz)
    echo "== unpack: ${ARTIFACT}"
    tar -xzf "$ARTIFACT" -C "$TMP"
    PKG=$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)
    [ -n "$PKG" ] || fail "${ARTIFACT} unpacked to no directory"
    ENGINE_REL="engine/duckdb"
    EXE="./brightfield"
    ;;
  *.dmg)
    [ "$(uname -s)" = "Darwin" ] || fail "a .dmg can only be attached on macOS"
    mkdir -p "$TMP/volume"
    echo "== attach: ${ARTIFACT}"
    attach_image "$ARTIFACT" "$TMP/volume" || fail "could not attach ${ARTIFACT}"
    MOUNT="$TMP/volume"
    PKG="$MOUNT"
    ENGINE_REL="Brightfield.app/Contents/Helpers/duckdb"
    EXE="./Brightfield.app/Contents/MacOS/brightfield"
    ;;
  *)
    fail "not an artifact this script knows: ${ARTIFACT} (expected .tar.gz or .dmg)"
    ;;
esac

[ -x "$PKG/$EXE" ] || fail "${ARTIFACT} carries no executable at ${EXE}"

[ -f "$PKG/$ENGINE_REL" ] || fail "${ARTIFACT} carries no engine at ${ENGINE_REL}.
  The Run control looks there and nowhere else, so on a machine with no duckdb
  installed every run would fail. scripts/package.sh stages it only when
  BRIGHTFIELD_DUCKDB_CLI is set."
[ -x "$PKG/$ENGINE_REL" ] || fail "the engine at ${ENGINE_REL} is not executable"

echo "== engine: ${ENGINE_REL}"
"${HERE}/fetch-duckdb-cli.sh" --check "$TARGET" "$(dirname "$PKG/$ENGINE_REL")" | sed 's/^/   /'

# The version the pin names, against the one Cargo.lock links. libduckdb-sys
# spells DuckDB 1.5.6 as 1.10506.0: the major, then the minor and patch as two
# digits each.
PINNED=$(sed -n 's/^VERSION="\(.*\)"$/\1/p' "$HERE/fetch-duckdb-cli.sh")
[ -n "$PINNED" ] || fail "scripts/fetch-duckdb-cli.sh has no VERSION= line"
SYS=$(awk '$0 == "name = \"libduckdb-sys\"" { getline; print }' "$ROOT/Cargo.lock" \
  | sed -n 's/^version = "1\.\([0-9]*\)\.0"$/\1/p')
[ -n "$SYS" ] || fail "Cargo.lock has no libduckdb-sys version of the form 1.<MmmPP>.0"
LINKED="v$((SYS / 10000)).$(((SYS / 100) % 100)).$((SYS % 100))"
[ "$PINNED" = "$LINKED" ] || fail "the staged engine is DuckDB ${PINNED} and the binary links
  DuckDB ${LINKED} (libduckdb-sys in Cargo.lock). A run would write with one and
  the window read with the other. Move the pin in scripts/fetch-duckdb-cli.sh
  with the embedded DuckDB."
echo "   the pin is DuckDB ${PINNED}, the version libduckdb-sys in Cargo.lock links"

ARTIFACT_NAME=$(basename "$ARTIFACT")
summary() {
  [ -n "${GITHUB_STEP_SUMMARY:-}" ] || return 0
  printf '%s\n' "$1" >> "$GITHUB_STEP_SUMMARY"
}

HOST=$("${HERE}/check-artifact-type-source.sh" --print-host-target)
if [ "$TARGET" = "$HOST" ]; then
  echo "== run: the packaged binary runs its engine"
  status=0
  ( cd "$PKG" && "$EXE" --check-engine ) > "$TMP/engine.log" 2>&1 || status=$?
  sed 's/^/   /' "$TMP/engine.log"
  case "$status" in
    0) summary "- engine RUN by the packaged binary: \`${ARTIFACT_NAME}\` (${TARGET}, DuckDB ${PINNED})" ;;
    2) fail "the binary reports no engine beside it, but ${ENGINE_REL} is in this artifact —
  it is staged somewhere the executable does not look. See scripts/package.sh." ;;
    *) fail "the packaged binary's engine did not check out (exit ${status}).
  It is present and the pinned bytes, and either it does not run or it is not the
  DuckDB the binary links. Read the message above." ;;
  esac
else
  echo "== not run: this artifact is for ${TARGET} and this machine is ${HOST},"
  echo "   so neither the packaged binary nor its engine can be executed here. The"
  echo "   engine was found and matched to its pin, and NOT run."
  echo "::warning title=Engine read but not run::${ARTIFACT_NAME} is packaged for ${TARGET} and this machine is ${HOST}, so its DuckDB engine was never executed. It was checked for its path and its pinned bytes only."
  summary "- engine READ BUT NOT RUN: \`${ARTIFACT_NAME}\` (packaged for ${TARGET}, this runner is ${HOST}) — path and pinned bytes were checked and the engine was never executed"
fi

COMPLETED=1
echo "check-artifact-engine: ${ARTIFACT} carries DuckDB ${PINNED} for ${TARGET}."
