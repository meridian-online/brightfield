#!/usr/bin/env bash
# Prove the packaged binary's air-gapped claim, against the artifact itself.
#
#   scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.tar.gz
#   scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.dmg
#
# Both artifacts of a darwin build get a leg, and the disk image's is not
# optional: without it the air-gapped proof runs against an artifact a download
# button never hands anybody.
#
# What "proof" means here, in order:
#
#   1. NEGATIVE CONTROL — the network denial actually denies. curl is run
#      inside the same jail the binary will run in and MUST fail; a jail that
#      lets curl through proves nothing about anything run inside it.
#   2. The artifact is opened where a stranger would open it — the tarball
#      unpacked into a fresh temp directory, the image attached read-only — and
#      the PACKAGED binary, not a repo build, opens, renders and screenshots
#      (a) a chart spec and (b) a Protocol manifest, entirely inside the jail,
#      with HOME and BRIGHTFIELD_CONFIG_DIR pointed into the temp directory so
#      nothing leaks in from this machine's config or out of the run.
#   3. Both screenshots are verified to be real PNGs of non-trivial size —
#      "it exited 0" is not "it rendered".
#   4. THE NEGATIVE CASE, which is the half a green run used to be silent
#      about. The artifact also carries specs that DO fetch (examples/remote/),
#      and the promise about those is not "they work" — it is that they fail
#      loudly, naming the network and the URL, and take nothing else down with
#      them. So one is run in the same jail and REQUIRED to fail: non-zero,
#      no PNG, and a message carrying both the word "network" and the URL. A
#      spec that needs the network and comes back quiet is the silent-degrade
#      defect this file exists to keep out of the artifact, and it would
#      otherwise be invisible here — the two positive legs above pass whether
#      or not it is present.
#
# The jail: macOS `sandbox-exec` with `(deny network*)`; Linux `unshare -rn`
# (a network namespace with no interfaces). Both windows open briefly on a
# desktop machine — the runs are real, that is the point.
#
# The image leg runs the bundle's executable directly. It does NOT use `open`:
# `open` was measured returning exit 0 while the application never started, and
# launchd reparents what it does start out of the jail, so an `open`-based check
# would report on a process this script is not confining.
#
# What this does NOT cover, for either artifact: Gatekeeper. Nothing here
# assesses a signature, a notarization ticket or the quarantine attribute, and a
# green run says nothing about them.
#
# Everything the test opens ships inside the artifact, so this script proves
# the artifact self-contained, not the artifact-plus-repo.
set -euo pipefail

ARTIFACT="${1:?usage: scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.(tar.gz|dmg)}"
[ -f "$ARTIFACT" ] || { echo "no such artifact: $ARTIFACT"; exit 1; }

case "$(uname -s)" in
  Darwin) JAIL=(sandbox-exec -p '(version 1)(allow default)(deny network*)') ;;
  Linux)  JAIL=(unshare -rn) ;;
  *) echo "no jail recipe for $(uname -s)"; exit 1 ;;
esac

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-airgap.XXXXXX")
MOUNT=""
cleanup() {
  # Detach before the temp tree goes: the mount point lives inside it, and
  # removing a directory an image is mounted on leaves the image attached.
  if [ -n "$MOUNT" ]; then hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || true; fi
  rm -rf "$TMP"
}
trap cleanup EXIT

# A window that never exits must be a FAILED check, not a stuck terminal.
DEADLINE=180

# smoke OUT.png [ENV=VAL ...] -- SPEC ARGS...
# Runs the packaged binary inside the jail, from the package directory, with a
# sealed HOME/config, a screenshot countdown, and the deadline watchdog.
# $PKG and $EXE are set by the leg that opened the artifact.
#
# stderr is TEED rather than swallowed: it stays on the terminal, and a copy
# lands in $SMOKE_LOG so a leg can assert on what the run said. The type-source
# leg needs that — "it rendered" is silent about whether the bundled extension
# and model came up inside the jail, and the application says so on stderr when
# they did not.
SMOKE_LOG=""
smoke() {
  local out="$1"; shift
  local extra_env=()
  while [ "$1" != "--" ]; do extra_env+=("$1"); shift; done
  shift
  SMOKE_LOG="$TMP/smoke-$(basename "$out").log"
  ( cd "$PKG" && env HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" \
      ${extra_env[@]+"${extra_env[@]}"} \
      "${JAIL[@]}" "$EXE" "$@" --shot-after 45 --shot-out "$out" \
      2> >(tee "$SMOKE_LOG" >&2) ) &
  local pid=$!
  ( sleep "$DEADLINE" && echo "   DEADLINE (${DEADLINE}s) — killing" && kill -9 "$pid" ) 2>/dev/null &
  local wd=$!
  local status=0
  wait "$pid" || status=$?
  kill "$wd" 2>/dev/null || true
  wait "$wd" 2>/dev/null || true
  [ "$status" -eq 0 ] || { echo "   FAILED: exit $status"; return 1; }
}

# refuses OUT.png LOG -- SPEC ARGS...
# The negative leg. Runs the packaged binary inside the same jail on a spec
# that needs the network, and passes only when it REFUSES: non-zero exit, no
# screenshot written, and a message naming the network and the location it
# could not reach.
#
# stderr is captured rather than shown, because here it is the evidence. It is
# echoed on failure, where it is the diagnosis.
refuses() {
  local out="$1" log="$2"; shift 2
  [ "$1" = "--" ] || { echo "   refuses(): expected -- before the spec"; return 1; }
  shift
  rm -f "$out"
  local status=0
  ( cd "$PKG" && env HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" \
      "${JAIL[@]}" "$EXE" "$@" --shot-after 45 --shot-out "$out" ) > "$log" 2>&1 &
  local pid=$!
  ( sleep "$DEADLINE" && echo "   DEADLINE (${DEADLINE}s) — killing" && kill -9 "$pid" ) 2>/dev/null &
  local wd=$!
  wait "$pid" || status=$?
  kill "$wd" 2>/dev/null || true
  wait "$wd" 2>/dev/null || true

  if [ "$status" -eq 0 ]; then
    echo "   FAILED: a spec that needs the network exited 0 with the network denied"
    sed 's/^/     /' "$log"
    return 1
  fi
  if [ -f "$out" ]; then
    echo "   FAILED: it refused, and still wrote $out — a picture of what?"
    return 1
  fi
  if ! grep -qi 'network' "$log"; then
    echo "   FAILED: it refused without naming the network:"
    sed 's/^/     /' "$log"
    return 1
  fi
  if ! grep -qF "$REMOTE_URL" "$log"; then
    echo "   FAILED: it refused without naming what it could not reach (${REMOTE_URL}):"
    sed 's/^/     /' "$log"
    return 1
  fi
  echo "   ok: refused (exit ${status}), naming the network and ${REMOTE_URL}"
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

# Each leg sets four things and nothing else: PKG (the directory to run from),
# EXE (the executable, relative to PKG), EXAMPLES (where the specs live,
# relative to PKG) and REMOTE_SPEC (the negative case's spec, likewise
# relative to PKG). Everything below is common.

# The negative case's subject, and the location its refusal has to name. The
# URL is written out rather than read from the spec so that a spec silently
# repointed at somewhere else fails this check instead of passing it against
# whatever it now says.
REMOTE_URL="https://openlake.meridian.online/edgar_gleif.parquet"
case "$ARTIFACT" in
  *.tar.gz)
    echo "== unpack: $ARTIFACT"
    tar -xzf "$ARTIFACT" -C "$TMP"
    PKG=$(find "$TMP" -mindepth 1 -maxdepth 1 -type d | head -1)
    mkdir -p "$TMP/home" "$TMP/config"
    EXE="./brightfield"
    EXAMPLES="examples"
    REMOTE_SPEC="examples/remote/edgar-gleif-crosswalk.yaml"
    FTBUNDLE="finetype"
    [ -x "$PKG/$EXE" ] || { echo "no executable 'brightfield' in the tarball"; exit 1; }
    ;;
  *.dmg)
    [ "$(uname -s)" = "Darwin" ] || { echo "a .dmg can only be attached on macOS"; exit 1; }
    mkdir -p "$TMP/home" "$TMP/config" "$TMP/volume"
    echo "== attach: $ARTIFACT"
    # -readonly and -noverify keep the attach from writing to the image or
    # spending a checksum pass on it; -nobrowse keeps it off the desktop.
    hdiutil attach -nobrowse -noverify -readonly -mountpoint "$TMP/volume" "$ARTIFACT"
    MOUNT="$TMP/volume"
    PKG="$MOUNT"
    EXE="./Brightfield.app/Contents/MacOS/brightfield"
    EXAMPLES="Brightfield.app/Contents/Resources/examples"
    REMOTE_SPEC="Brightfield.app/Contents/Resources/examples/remote/edgar-gleif-crosswalk.yaml"
    FTBUNDLE="Brightfield.app/Contents/Resources/finetype"
    [ -x "$PKG/$EXE" ] || { echo "no executable inside Brightfield.app on the image"; exit 1; }
    ;;
  *)
    echo "not an artifact this script knows: $ARTIFACT (expected .tar.gz or .dmg)"
    exit 1
    ;;
esac

# The semantic type source, if this artifact carries one. Read off the files
# before anything is run, because a bundle that is incomplete or full of
# dangling symlinks would otherwise show up only as a column with no label —
# which is also what a build packaged deliberately without one looks like.
#
# HAS_TYPE_SOURCE is what run 1 below asserts against. It is set to 0 for an
# artifact with no bundle, which is a supported build (see scripts/package.sh);
# a bundle that is present and broken is a FAILURE, not an absence.
HAS_TYPE_SOURCE=0
if [ ! -d "$PKG/$FTBUNDLE" ]; then
  echo "== type source: this artifact carries none — the label legs are skipped"
else
  echo "== type source: ${FTBUNDLE}"
  for required in finetype.duckdb_extension model/model.safetensors model/config.json \
                  model/label_map.json model/model2vec/model.safetensors \
                  model/model2vec/tokenizer.json taxonomy-schemas.json; do
    [ -f "$PKG/$FTBUNDLE/$required" ] || {
      echo "   FAILED: the bundle carries no ${required}"; exit 1; }
  done
  # Self-containedness. `cp -RL` in scripts/package.sh is what makes a
  # cache-fetched model portable, and a symlink surviving into the artifact is
  # the exact failure it prevents: it resolves on the packaging machine and
  # dangles everywhere else, so it cannot be caught by running the artifact
  # where it was built.
  strays=$(find "$PKG/$FTBUNDLE" -type l | wc -l | tr -d ' ')
  [ "$strays" -eq 0 ] || {
    echo "   FAILED: ${strays} symlink(s) inside the bundle — it is not self-contained"
    find "$PKG/$FTBUNDLE" -type l | sed 's/^/     /'
    exit 1; }
  # The metadata trailer, read the same way scripts/package.sh and
  # brightfield_engine::semantic read it: last 512 bytes, field 1 (magic) at
  # offset 224, ABI at 96, platform at 192.
  ft_field() {
    tail -c 512 "$PKG/$FTBUNDLE/finetype.duckdb_extension" \
      | dd bs=1 skip="$1" count=32 2>/dev/null | tr -d '\0'
  }
  [ "$(ft_field 224)" = "4" ] || {
    echo "   FAILED: the bundled extension carries no DuckDB metadata trailer"; exit 1; }
  [ "$(ft_field 96)" = "C_STRUCT" ] || {
    echo "   FAILED: the bundled extension is not a stable-C-API (C_STRUCT) build"; exit 1; }
  echo "   ok: finetype $(ft_field 128), $(ft_field 96), $(ft_field 192), no symlinks"
  HAS_TYPE_SOURCE=1
fi

echo "== run 1: chart spec, jailed (a window opens briefly)"
smoke "$TMP/chart.png" -- "$EXAMPLES/bars.yaml"
is_png "$TMP/chart.png" 20000

# THE AIR-GAPPED HALF OF THE TYPE-SOURCE CLAIM, and the only place it is
# actually proved. The file checks above say the bytes are present; this says
# the extension LOADED and its model CLASSIFIED, inside the network-denied jail,
# with HOME and the config directory pointed at an empty temp tree so no warm
# cache from this machine can be reached.
#
# It works by negation because there is no headless way to read a label out of
# a GUI binary: the application prints `warning: no semantic type source` and
# the reason whenever a configured bundle fails to come up, and coming up
# includes a canary that makes the model classify three email addresses. Silence
# on that line, from a run that also rendered, is the evidence.
if [ "$HAS_TYPE_SOURCE" -eq 1 ]; then
  echo "== run 1b: the type source came up inside the jail"
  if grep -q 'no semantic type source' "$SMOKE_LOG"; then
    echo "   FAILED: the bundled type source did not come up with the network denied:"
    grep 'no semantic type source' "$SMOKE_LOG" | sed 's/^/     /'
    exit 1
  fi
  echo "   ok: no type-source warning from a run that rendered"
fi

echo "== run 2: Protocol manifest, jailed (a window opens briefly)"
smoke "$TMP/protocol.png" BRIGHTFIELD_PROTOCOL_OFFLINE=1 -- "$EXAMPLES/protocol/edgar_gleif/arcform.yaml"
is_png "$TMP/protocol.png" 20000

echo "== run 3: a spec that needs the network, jailed — it must REFUSE"
[ -f "$PKG/$REMOTE_SPEC" ] || {
  echo "   FAILED: the artifact carries no $REMOTE_SPEC, so the negative case"
  echo "   cannot be run against it. See scripts/package.sh."
  exit 1
}
refuses "$TMP/remote.png" "$TMP/remote.log" -- "$REMOTE_SPEC"

echo "== run 4: the local chart again, AFTER the refusal — the jail is unchanged"
smoke "$TMP/chart-again.png" -- "$EXAMPLES/bars.yaml"
is_png "$TMP/chart-again.png" 20000

echo "== PASS: the packaged binary starts, renders and opens a local protocol with the"
echo "         network denied — and refuses a remote spec by name without disturbing either"
