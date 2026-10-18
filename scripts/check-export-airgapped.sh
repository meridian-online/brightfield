#!/usr/bin/env bash
# Prove a Brightfield binary runs an agent-written Protocol and exports its
# report to PDF with the network denied.
#
#   scripts/check-export-airgapped.sh <brightfield> [<duckdb-cli>]
#
# scripts/verify-airgapped.sh runs this against the PACKAGED binary; it can be
# pointed at any build (a `cargo build` one, say) to try the leg on its own.
#
# The Protocol is the committed fixture tests/fixtures/agent-protocols/
# co2-per-capita, an agent's Protocol kept as written, with its data file cut
# down. It comes from this checkout rather than from the artifact, which ships
# no fixtures: the claim is about the binary, given a Protocol on disk.
#
# In order, all inside the same jail verify-airgapped.sh uses:
#
#   1. NEGATIVE CONTROL: curl must fail in the jail, or nothing below proves
#      anything about the network.
#   2. A copy of the fixture has its http_fetch step dropped
#      (scripts/drop-fetch-steps.sh), through the binary's own arc. arc fetches
#      again whenever a step pins no sha256, which would need the network and
#      would replace the cut-down file; dropping the step is how the test suite
#      runs the same fixture.
#   3. The binary, in its runner mode, runs the copy with arc (`arc run`), the
#      way the window's Run control does, with the DuckDB CLI given or staged
#      beside the binary. The data file must be byte-identical afterwards.
#   4. `export-pdf` writes the report, and it must exit 0 and leave a PDF of
#      non-trivial size.
#
# HOME, BRIGHTFIELD_CONFIG_DIR and arc's database and history stores are all
# pointed into a temp directory, so nothing is read from or written to this
# machine's own.
#
# The DuckDB CLI: the second argument; else the one a package stages beside the
# binary (engine/duckdb in the tarball, Contents/Helpers/duckdb in the app),
# the places the binary itself looks; else $ARC_DUCKDB_BIN.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
EXE="${1:?usage: scripts/check-export-airgapped.sh <brightfield> [<duckdb-cli>]}"
[ -x "$EXE" ] || { echo "not an executable: $EXE"; exit 1; }
EXE=$(cd "$(dirname "$EXE")" && pwd)/$(basename "$EXE")

ENGINE="${2:-}"
if [ -z "$ENGINE" ]; then
  for candidate in "$(dirname "$EXE")/engine/duckdb" "$(dirname "$EXE")/../Helpers/duckdb"; do
    if [ -x "$candidate" ]; then ENGINE="$candidate"; break; fi
  done
fi
ENGINE="${ENGINE:-${ARC_DUCKDB_BIN:-}}"
[ -n "$ENGINE" ] && [ -x "$ENGINE" ] || {
  echo "FAILED: no DuckDB CLI to run the Protocol with: none given, none staged beside"
  echo "$EXE, and ARC_DUCKDB_BIN names none (scripts/fetch-duckdb-cli.sh writes one)"
  exit 1
}
# arc runs from inside the Protocol's folder, so a relative path would miss.
ENGINE=$(cd "$(dirname "$ENGINE")" && pwd)/$(basename "$ENGINE")

FIXTURE="$ROOT/tests/fixtures/agent-protocols/co2-per-capita"
DATA="data/owid-co2-data.csv"
[ -d "$FIXTURE" ] || { echo "FAILED: no fixture at $FIXTURE"; exit 1; }

case "$(uname -s)" in
  Darwin) JAIL=(sandbox-exec -p '(version 1)(allow default)(deny network*)') ;;
  Linux)  JAIL=(unshare -rn) ;;
  *) echo "no jail recipe for $(uname -s)"; exit 1 ;;
esac

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-export-airgap.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/home" "$TMP/config" "$TMP/arc-db" "$TMP/arc-history"
export HOME="$TMP/home" BRIGHTFIELD_CONFIG_DIR="$TMP/config" \
  ARCFORM_DB_DIR="$TMP/arc-db" ARCFORM_HISTORY_DIR="$TMP/arc-history" \
  ARC_DUCKDB_BIN="$ENGINE" NO_COLOR=1

echo "== negative control: the jail must deny the network"
command -v curl >/dev/null || { echo "curl is required (for the control, not the binary)"; exit 1; }
if "${JAIL[@]}" curl -sS --max-time 10 https://example.com -o /dev/null 2>/dev/null; then
  echo "   FAILED: curl reached the network inside the jail — this jail proves nothing"
  exit 1
fi
echo "   ok: curl cannot reach the network in the jail"

COPY="$TMP/co2-per-capita"
cp -R "$FIXTURE" "$COPY"
ARC=("${JAIL[@]}" env BRIGHTFIELD_RUN_AS_ARC=1 "$EXE")

echo "== drop the fetch step from a copy of the fixture, jailed"
"$ROOT/scripts/drop-fetch-steps.sh" "$COPY" "${ARC[@]}" | sed 's/^/   /'

echo "== arc run, through the binary's runner mode, jailed"
before=$(cksum < "$COPY/$DATA")
if ! ( cd "$COPY" && "${ARC[@]}" run ) > "$TMP/run.log" 2>&1; then
  echo "   FAILED: the run did not succeed with the network denied:"
  sed 's/^/     /' "$TMP/run.log"
  exit 1
fi
[ "$(cksum < "$COPY/$DATA")" = "$before" ] || {
  echo "   FAILED: the run replaced $DATA, so something fetched it"; exit 1; }
echo "   ok: the run succeeded and left $DATA as it was"

echo "== export-pdf, jailed (no window)"
PDF="$TMP/report.pdf"
status=0
"${JAIL[@]}" "$EXE" export-pdf "$COPY" --out "$PDF" > "$TMP/export.log" 2>&1 || status=$?
if [ "$status" -ne 0 ]; then
  echo "   FAILED: export-pdf exited $status with the network denied:"
  sed 's/^/     /' "$TMP/export.log"
  exit 1
fi
[ -f "$PDF" ] || { echo "   FAILED: export-pdf exited 0 and wrote no $PDF"; exit 1; }
magic=$(head -c 4 "$PDF" | od -An -tx1 | tr -d ' \n')
[ "$magic" = "25504446" ] || { echo "   FAILED: $PDF is not a PDF"; exit 1; }
size=$(wc -c < "$PDF" | tr -d ' ')
[ "$size" -ge 50000 ] || { echo "   FAILED: $PDF is suspiciously small (${size}B)"; exit 1; }
sed -e '/^[[:space:]]*$/d' -e 's/^/   /' "$TMP/export.log"
echo "   ok: report.pdf (${size}B), exported with the network denied"
