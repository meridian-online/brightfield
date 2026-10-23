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
#      (a) a chart spec, (b) a Protocol manifest and (c) a Parquet data file,
#      which needs DuckDB's Parquet reader linked in, entirely inside the jail,
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
#   5. THE SEMANTIC TYPE SOURCE, when the artifact carries one. First the
#      bundle is read off disk by scripts/check-bundled-extension.sh — shape,
#      ABI, and a symlink count, because a model copied with `cp -R` instead of
#      `cp -RL` produces an artifact that works only on the machine that built
#      it and can never be caught by running it there. Then the half no file
#      check can see: the packaged binary is run with `--check-type-source`
#      inside the same jail, which loads that extension with its own DuckDB,
#      loads the model beside it, and puts a label on a column — and reports it
#      as an EXIT CODE. Nothing here is inferred from the absence of a message.
#      That flag seals its own environment and working directory before it does
#      any of it, so the verdict is not about what this machine happened to have
#      exported; this script does not have to arrange that and could not do it
#      completely if it tried.
#
#      THE FILESYSTEM IS JAILED TOO, on macOS. The leg runs under the read jail
#      in scripts/airgap-jail.sh, which denies the network and refuses to read
#      the contents of any file outside this run's temp directory (the unpacked
#      artifact, the sealed HOME, and the sealed working directory the flag
#      makes for itself) and the system paths dyld and DuckDB need, each granted
#      by name from a recorded run of this leg. So a green run here also means
#      nothing outside the artifact was read to type that column. Proving that
#      by naming the ways out was an enumeration, wrong three times running —
#      about a code path, an environment variable, and a working directory —
#      and deny-by-default is right about a way out nobody has named.
#
#      Two things keep that claim honest. scripts/airgap-jail-selftest.sh runs
#      on every pull request and proves the jail refuses a read outside its
#      grant. And run 0b below plants the one route known to be open — an
#      absolute `config.value_embed_model`, which FineType resolves against the
#      model directory and then takes as written — in a copy of the artifact,
#      and requires the network-only jail to let it through and the read jail
#      to stop it. On Linux there is no read jail (`unshare -rn` is a network
#      namespace), and the leg says so: there it is a smoke test of the type
#      source, not a containment proof.
#      An artifact with NO bundle skips this leg (a supported build, see
#      scripts/package.sh); a bundle that is present and does not work is a
#      failure.
#   6. THE HEADLESS EXPORT. The packaged binary runs an agent-written Protocol
#      with arc (through its runner mode and the DuckDB CLI the artifact
#      stages) and writes its report with `export-pdf`, all inside the jail,
#      and must leave a PDF. The Protocol is the repository's committed
#      fixture with its fetch step dropped from a copy, since arc would
#      download the data again; scripts/check-export-airgapped.sh holds the
#      leg and names what it checks. The DuckDB CLI the run needs is the one
#      the artifact stages and no other; an artifact that stages none skips
#      this leg, saying so.
#
# The jail: macOS `sandbox-exec` with `(deny network*)`; Linux `unshare -rn`
# (a network namespace with no interfaces). Both windows open briefly on a
# desktop machine — the runs are real, that is the point. Leg 5 runs under a
# stricter jail on macOS, which also refuses reads outside the artifact: see
# scripts/airgap-jail.sh. The window-opening legs do not, yet.
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
# Every SPEC the test opens ships inside the artifact, so the render legs are
# about the artifact rather than the artifact-plus-repo. That is a statement
# about the specs. The type-source leg's claim is about the artifact on macOS,
# where its jail refuses reads outside it, and is not on Linux: see leg 5,
# which names precisely what it does and does not establish. The export leg,
# 6, takes its Protocol from this checkout: the artifact ships no fixtures, and
# that leg's claim is about the binary given a Protocol on disk.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ARTIFACT="${1:?usage: scripts/verify-airgapped.sh dist/brightfield-<version>-<target>.(tar.gz|dmg)}"
[ -f "$ARTIFACT" ] || { echo "no such artifact: $ARTIFACT"; exit 1; }

# shellcheck source=scripts/airgap-jail.sh
. "${HERE}/airgap-jail.sh"

case "$(uname -s)" in
  Darwin) JAIL=(sandbox-exec -p '(version 1)(allow default)(deny network*)') ;;
  Linux)  JAIL=(unshare -rn) ;;
  *) echo "no jail recipe for $(uname -s)"; exit 1 ;;
esac

# Resolved, because the read jail grants this directory and the sandbox matches
# real paths: mktemp's answer is behind /var -> /private/var on macOS.
TMP=$(cd "$(mktemp -d "${TMPDIR:-/tmp}/bf-airgap.XXXXXX")" && pwd -P)
# Outside TMP on purpose: run 0b moves part of a model here so that reading it
# means reading outside the artifact.
OUTSIDE=""
MOUNT=""
cleanup() {
  # Detach before the temp tree goes: the mount point lives inside it, and
  # removing a directory an image is mounted on leaves the image attached.
  if [ -n "$MOUNT" ]; then hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || true; fi
  rm -rf "$TMP"
  if [ -n "$OUTSIDE" ]; then rm -rf "$OUTSIDE"; fi
}
trap cleanup EXIT

# A window that never exits must be a FAILED check, not a stuck terminal.
DEADLINE=180

# smoke OUT.png [ENV=VAL ...] -- SPEC ARGS...
# Runs the packaged binary inside the jail, from the package directory, with a
# sealed HOME/config, a screenshot countdown, and the deadline watchdog.
# $PKG and $EXE are set by the leg that opened the artifact.
smoke() {
  local out="$1"; shift
  local extra_env=()
  while [ "$1" != "--" ]; do extra_env+=("$1"); shift; done
  shift
  ( cd "$PKG" && env HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" \
      ${extra_env[@]+"${extra_env[@]}"} \
      "${JAIL[@]}" "$EXE" "$@" --shot-after 45 --shot-out "$out" ) &
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
  # One reading of the bundle, shared with scripts/package.sh — including the
  # symlink count, which is the check that a `cp -R` where `cp -RL` was needed
  # produces an artifact working ONLY on the machine that built it. No platform
  # argument: the artifact does not know which target it is, and the packaging
  # run already decided that against the target it was building for.
  "${HERE}/check-bundled-extension.sh" "$PKG/$FTBUNDLE" | sed 's/^/   /' || {
    echo "   FAILED: the artifact carries a type source that would not load"; exit 1; }
  HAS_TYPE_SOURCE=1
fi

# THE AIR-GAPPED HALF OF THE TYPE-SOURCE CLAIM. The file checks above say the
# bytes are present; this says the PACKAGED BINARY can load that extension with
# its own DuckDB, load the model beside it, and put a label on a column, with
# the network denied.
#
# WHERE THE INPUTS COME FROM IS THE BINARY'S JOB, NOT THIS SCRIPT'S, and that
# division is the correction of a defect rather than a preference. Redirecting
# HOME here — which this script does — closes the HuggingFace cache and leaves
# a larger door open: with FINETYPE_MODEL_DIR exported in whatever environment
# the script was invoked from, an earlier version of this leg reported success
# on a bundle whose weights were eighteen bytes of rubbish, because the
# extension took its model from there. Neither sandbox-exec nor unshare -rn
# scrubs the environment, and `env HOME=…` without -i does not either.
#
# So --check-type-source re-execs itself with the environment CLEARED and only
# what it needs put back, and refuses to report at all if it sees anything
# outside that list and the variables the platform writes into the child after
# execve. This script therefore passes it whatever environment it likes and
# trusts the EXIT CODE, which is what the flag exists to produce.
#
# The version of this leg that shipped first grepped a rendering run's stderr
# for a warning and passed on its absence — but the rendering path never
# configures a type source, so the warning could not appear and the leg passed
# for any bundle at all. A check whose failing case cannot be reached is not a
# check; so is one whose inputs it cannot account for.
#
# `--check-type-source` opens no window: it is the one leg here that can be run
# unattended, and the one that can be made to fail on demand by planting a
# broken bundle in the artifact.
# THE READ JAIL, and where the flag's sealed directory goes. --check-type-source
# makes its sealed HOME, TMPDIR and working directory under the temp directory
# it inherits, at a name it picks at run time, so no profile could grant that
# directory by name. TMPDIR is therefore pointed inside TMP for these runs, and
# TMP is what the jail grants.
mkdir -p "$TMP/sealed-tmp"
if [ "$(uname -s)" = "Darwin" ]; then
  TS_JAIL=(sandbox-exec -p "$(read_jail_profile "$TMP")")
  TS_JAIL_SAYS="reads outside the artifact refused"
else
  TS_JAIL=("${JAIL[@]}")
  TS_JAIL_SAYS="NO READ JAIL on $(uname -s): network denied, filesystem open"
fi

# type_source DIR LOG JAIL... — run DIR's packaged binary's --check-type-source
# under JAIL, stdout and stderr to LOG, and echo its exit status.
type_source() {
  local dir="$1" log="$2"; shift 2
  local status=0
  ( cd "$dir" && env HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" TMPDIR="$TMP/sealed-tmp" \
      "$@" "$EXE" --check-type-source ) > "$log" 2>&1 || status=$?
  echo "$status"
}

if [ "$HAS_TYPE_SOURCE" -eq 1 ]; then
  echo "== run 0: the packaged binary types a column, jailed (no window; ${TS_JAIL_SAYS})"
  ts_status=$(type_source "$PKG" "$TMP/typesource.log" "${TS_JAIL[@]}")
  sed 's/^/   /' "$TMP/typesource.log"
  case "$ts_status" in
    0) echo "   ok: the bundled extension and model typed a column with no network" ;;
    2) echo "   FAILED: the binary found no bundle, but ${FTBUNDLE} is in this artifact —"
       echo "   it is staged somewhere the executable does not look. See scripts/package.sh."
       exit 1 ;;
    *) echo "   FAILED: the bundled type source did not come up (exit ${ts_status})"
       if [ "$(uname -s)" = "Darwin" ]; then
         echo "   If the message above names a file it could not open, the artifact reads"
         echo "   outside itself — or the binary needs a system path scripts/airgap-jail.sh"
         echo "   does not grant. Record a run before granting one; see that file."
       fi
       exit 1 ;;
  esac
fi

# RUN 0b: THE KNOWN WAY OUT, PLANTED, and the read jail required to stop it.
#
# A copy of the unpacked artifact, with the model's value encoder moved outside
# TMP and the model config pointed at it by absolute path — the route FineType
# leaves open, since it resolves `value_embed_model` against the model
# directory and then falls back to the path as written. The manifest goes from
# the copy, because it records the config it was written over, and a bundle
# carrying none is one the flag accepts.
#
# Two runs of the copy, and each one is half of the proof:
#   - under the network-only jail it MUST PASS. That shows the copy really reads
#     its encoder from outside the artifact; if it fails, FineType no longer
#     honours the path, the case proves nothing, and it says so.
#   - under the read jail it MUST FAIL. That is the jail stopping a bundle that
#     works on the machine that built it and loses its labels on a stranger's.
if [ "$HAS_TYPE_SOURCE" -eq 1 ] && [ "$(uname -s)" = "Darwin" ]; then
  echo "== run 0b: a bundle whose model reads outside the artifact, jailed — it must be REFUSED"
  command -v python3 >/dev/null || { echo "   FAILED: python3 is required to rewrite the model config"; exit 1; }
  OUTSIDE=$(cd "$(mktemp -d "${TMPDIR:-/tmp}/bf-airgap-outside.XXXXXX")" && pwd -P)
  cp -R "$PKG" "$TMP/escape"
  ESC_MODEL="$TMP/escape/${FTBUNDLE}/model"
  ENCODER=$(python3 -I -c 'import json,sys; print(json.load(open(sys.argv[1]))["value_embed_model"])' "$ESC_MODEL/config.json")
  case "$ENCODER" in
    /*) echo "   FAILED: the shipped model config already names an absolute encoder (${ENCODER})"; exit 1 ;;
  esac
  [ -d "$ESC_MODEL/$ENCODER" ] || { echo "   FAILED: no encoder at ${FTBUNDLE}/model/${ENCODER} to move"; exit 1; }
  mv "$ESC_MODEL/$ENCODER" "$OUTSIDE/$ENCODER"
  python3 -I -c 'import json,sys
p, enc = sys.argv[1], sys.argv[2]
c = json.load(open(p)); c["value_embed_model"] = enc
json.dump(c, open(p, "w"), indent=2)' "$ESC_MODEL/config.json" "$OUTSIDE/$ENCODER"
  rm -f "$TMP/escape/${FTBUNDLE}/bundle-manifest.sha256"

  esc_open=$(type_source "$TMP/escape" "$TMP/escape-open.log" "${JAIL[@]}")
  if [ "$esc_open" -ne 0 ]; then
    echo "   FAILED: the planted bundle did not come up even with the filesystem open (exit ${esc_open}),"
    echo "   so refusing it under the read jail would prove nothing. Either FineType no longer"
    echo "   follows an absolute value_embed_model, and this case wants rewriting, or the copy"
    echo "   is broken:"
    sed 's/^/     /' "$TMP/escape-open.log"
    exit 1
  fi
  echo "   ok: with only the network denied, it typed a column reading ${OUTSIDE}/${ENCODER}"
  esc_jailed=$(type_source "$TMP/escape" "$TMP/escape-jailed.log" "${TS_JAIL[@]}")
  if [ "$esc_jailed" -eq 0 ]; then
    echo "   FAILED: the read jail let a bundle read its encoder from outside the artifact"
    sed 's/^/     /' "$TMP/escape-jailed.log"
    exit 1
  fi
  echo "   ok: refused under the read jail (exit ${esc_jailed}):"
  sed 's/^/     /' "$TMP/escape-jailed.log"
  rm -rf "$TMP/escape"
fi

echo "== run 1: chart spec, jailed (a window opens briefly)"
smoke "$TMP/chart.png" -- "$EXAMPLES/bars.yaml"
is_png "$TMP/chart.png" 20000

echo "== run 2: Protocol manifest, jailed (a window opens briefly)"
smoke "$TMP/protocol.png" BRIGHTFIELD_PROTOCOL_OFFLINE=1 -- "$EXAMPLES/protocol/edgar_gleif/arcform.yaml"
is_png "$TMP/protocol.png" 20000

echo "== run 2b: a Parquet data file, jailed (a window opens briefly)"
# DuckDB reads Parquet through an extension it autoinstalls on first use unless
# the reader is linked into the binary. HOME is sealed above, so the extension
# cache is cold, and the jail denies the network: a build without the reader
# fails here downloading it, where every chart-spec leg stays green.
smoke "$TMP/parquet.png" -- "$EXAMPLES/data/california_housing.parquet"
is_png "$TMP/parquet.png" 20000
# One command, not `find | grep -q`: under pipefail, grep -q exiting at its first
# match can leave find dying of SIGPIPE, and the pipeline's 141 would read as
# "nothing installed".
if [ -n "$(find "$TMP/home" -name '*.duckdb_extension' 2>/dev/null)" ]; then
  echo "   FAILED: opening a Parquet installed a DuckDB extension into the sealed HOME"
  exit 1
fi

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

echo "== run 5: an agent-written Protocol run and exported to PDF, jailed (no window)"
# The packaged binary, its staged DuckDB CLI and the committed fixture: see
# scripts/check-export-airgapped.sh, which can also be pointed at a repo build.
# --packaged: the engine comes only from the artifact (never the caller's
# ARC_DUCKDB_BIN or PATH), and an artifact that stages none skips the leg.
"${HERE}/check-export-airgapped.sh" --packaged "$PKG/${EXE#./}" | sed 's/^/   /' || {
  echo "   FAILED: the packaged binary could not run and export a Protocol with the network denied"
  exit 1
}

echo "== PASS: the packaged binary starts, renders and opens a local protocol with the"
echo "         network denied, runs and exports an agent-written Protocol to PDF the same way"
echo "         (unless run 5 said it was skipped), and refuses a remote spec by name"
