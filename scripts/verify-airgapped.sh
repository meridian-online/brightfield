#!/usr/bin/env bash
# Prove the packaged binary's air-gapped claim, against the artifact itself.
#
#   scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.tar.gz
#
# What "proof" means here, in order:
#
#   1. NEGATIVE CONTROL — the network denial actually denies. curl is run
#      inside the same jail the binary will run in and MUST fail; a jail that
#      lets curl through proves nothing about anything run inside it.
#   2. The tarball is unpacked into a fresh temp directory and the PACKAGED
#      binary — not a repo build — opens, renders and screenshots
#      (a) a chart spec and (b) a Protocol manifest, entirely inside the jail,
#      with HOME and BRIGHTFIELD_CONFIG_DIR pointed into the temp directory so
#      nothing leaks in from this machine's config or out of the run.
#   3. Both screenshots are verified to be real PNGs of non-trivial size —
#      "it exited 0" is not "it rendered".
#
# The jail: macOS `sandbox-exec` with `(deny network*)`; Linux `unshare -rn`
# (a network namespace with no interfaces). Both windows open briefly on a
# desktop machine — the runs are real, that is the point.
#
# Everything the test opens ships inside the tarball, so this script proves
# the artifact self-contained, not the artifact-plus-repo.
set -euo pipefail

TARBALL="${1:?usage: scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.tar.gz}"
[ -f "$TARBALL" ] || { echo "no such tarball: $TARBALL"; exit 1; }

case "$(uname -s)" in
  Darwin) JAIL=(sandbox-exec -p '(version 1)(allow default)(deny network*)') ;;
  Linux)  JAIL=(unshare -rn) ;;
  *) echo "no jail recipe for $(uname -s)"; exit 1 ;;
esac

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-airgap.XXXXXX")
trap 'rm -rf "$TMP"' EXIT

# A window that never exits must be a FAILED check, not a stuck terminal.
DEADLINE=180

# smoke OUT.png [ENV=VAL ...] -- SPEC ARGS...
# Runs the packaged binary inside the jail, from the package directory, with a
# sealed HOME/config, a screenshot countdown, and the deadline watchdog.
smoke() {
  local out="$1"; shift
  local extra_env=()
  while [ "$1" != "--" ]; do extra_env+=("$1"); shift; done
  shift
  ( cd "$PKG" && env HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" \
      ${extra_env[@]+"${extra_env[@]}"} \
      "${JAIL[@]}" ./brightfield "$@" --shot-after 45 --shot-out "$out" ) &
  local pid=$!
  ( sleep "$DEADLINE" && echo "   DEADLINE (${DEADLINE}s) — killing" && kill -9 "$pid" ) 2>/dev/null &
  local wd=$!
  local status=0
  wait "$pid" || status=$?
  kill "$wd" 2>/dev/null || true
  wait "$wd" 2>/dev/null || true
  [ "$status" -eq 0 ] || { echo "   FAILED: exit $status"; return 1; }
}

is_png() {
  local f="$1" min_bytes="$2"
  [ -f "$f" ] || { echo "   MISSING: $f"; return 1; }
  local magic size
  magic=$(head -c 4 "$f" | od -An -tx1 | tr -d ' \n')
  size=$(wc -c < "$f" | tr -d ' ')
  [ "$magic" = "89504e47" ] || { echo "   NOT A PNG: $f"; return 1; }
  [ "$size" -ge "$min_bytes" ] || { echo "   SUSPICIOUSLY SMALL (${size}B): $f"; return 1; }
  echo "   ok: $f (${size}B)"
}

echo "== negative control: the jail must deny the network"
command -v curl >/dev/null || { echo "curl is required (for the control, not the binary)"; exit 1; }
if "${JAIL[@]}" curl -sS --max-time 10 https://example.com -o /dev/null 2>/dev/null; then
  echo "   FAILED: curl reached the network inside the jail — this jail proves nothing"
  exit 1
fi
echo "   ok: curl cannot reach the network in the jail"

echo "== unpack: $TARBALL"
tar -xzf "$TARBALL" -C "$TMP"
PKG=$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)
mkdir -p "$TMP/home" "$TMP/config"
[ -x "$PKG/brightfield" ] || { echo "no executable 'brightfield' in the tarball"; exit 1; }

echo "== run 1: chart spec, jailed (a window opens briefly)"
smoke "$TMP/chart.png" -- examples/bars.yaml
is_png "$TMP/chart.png" 20000

echo "== run 2: Protocol manifest, jailed (a window opens briefly)"
smoke "$TMP/protocol.png" BRIGHTFIELD_PROTOCOL_OFFLINE=1 -- examples/protocol/edgar_gleif/arcform.yaml
is_png "$TMP/protocol.png" 20000

echo "== PASS: the packaged binary starts, renders and opens a local protocol with the network denied"
