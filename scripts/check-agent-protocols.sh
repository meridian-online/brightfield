#!/usr/bin/env bash
# Draw every panel and export every report of a set of agent-written Protocols.
#
#   scripts/check-agent-protocols.sh <runs-dir> [<out-dir>]
#
# Every folder under <runs-dir> holding an arcform.yaml and a panels/ folder is
# a Protocol to check. Each is COPIED into a temp directory first, because a run
# writes into its folder, and the originals are left as they are. In the copy:
#
#   1. Its http_fetch steps are dropped when their files are already there
#      (scripts/drop-fetch-steps.sh), so the check needs no network and cannot
#      replace the data the agent fetched.
#   2. `arc run` runs, unless the copy already has a database in this check's
#      store. arc keys a database by the folder's path, so a copy never has one
#      to begin with, and the run records the agent's runs left in the folder
#      say nothing about this store. They are kept, so the report's run is
#      still found next to the new one.
#   3. `export-png` for each panels/*.yaml, and `export-pdf` when there is a
#      report.md.
#
# Prints one line per panel and report, `ok <what>` or `FAIL <what>: <message>`,
# with any warning an export printed indented under it, then the counts. Exits 1
# on any failure.
#
# The pictures go to <out-dir>, one folder per Protocol, when it is given, and
# are removed with the temp directory otherwise.
#
# Environment:
#   BRIGHTFIELD_BIN  the Brightfield binary (default: target/debug/brightfield-shell)
#   ARC_BIN          the arc executable (default: BRIGHTFIELD_BIN in its runner
#                    mode, which carries the arc Brightfield is pinned to)
#   ARC_DUCKDB_BIN   the DuckDB CLI arc runs steps with (default:
#                    target/duckdb-cli/duckdb when it is there, which
#                    scripts/fetch-duckdb-cli.sh writes)
#
# arc's database and history stores are pointed into the temp directory, so
# nothing is written to ~/.arcform.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
RUNS="${1:?usage: scripts/check-agent-protocols.sh <runs-dir> [<out-dir>]}"
[ -d "$RUNS" ] || { echo "no such directory: $RUNS" >&2; exit 2; }
OUT="${2:-}"

BF="${BRIGHTFIELD_BIN:-$ROOT/target/debug/brightfield-shell}"
[ -x "$BF" ] || { echo "no Brightfield binary at $BF (build it, or set BRIGHTFIELD_BIN)" >&2; exit 2; }
if [ -n "${ARC_BIN:-}" ]; then
  ARC=("$ARC_BIN")
else
  ARC=(env BRIGHTFIELD_RUN_AS_ARC=1 "$BF")
fi
if [ -z "${ARC_DUCKDB_BIN:-}" ] && [ -x "$ROOT/target/duckdb-cli/duckdb" ]; then
  export ARC_DUCKDB_BIN="$ROOT/target/duckdb-cli/duckdb"
fi

TMP=$(mktemp -d "${TMPDIR:-/tmp}/bf-agent-protocols.XXXXXX")
trap 'rm -rf "$TMP"' EXIT
export ARCFORM_DB_DIR="$TMP/arc-db" ARCFORM_HISTORY_DIR="$TMP/arc-history" NO_COLOR=1
mkdir -p "$ARCFORM_DB_DIR" "$ARCFORM_HISTORY_DIR"

protocols=0 panels_ok=0 panels_failed=0 pdfs_ok=0 pdfs_failed=0 protocols_failed=0

# fail WHAT LOG: one FAIL line, carrying the log's lines joined.
fail() {
  local message
  message=$(sed -e 's/^[[:space:]]*//' -e '/^$/d' "$2" | paste -sd ' ' -)
  echo "FAIL $1: ${message:-no message}"
}

# The log's lines, indented, under an ok line: an export's warnings.
notes() { sed -e '/^[[:space:]]*$/d' -e 's/^/    /' "$1"; }

# magic FILE HEX: the file starts with these bytes.
magic() {
  [ -f "$1" ] && [ "$(head -c $(( ${#2} / 2 )) "$1" | od -An -tx1 | tr -d ' \n')" = "$2" ]
}

while IFS= read -r manifest; do
  src=$(dirname "$manifest")
  [ -d "$src/panels" ] || continue
  protocols=$((protocols + 1))
  rel=${src#"$RUNS"/}
  name=$(basename "$src")
  work="$TMP/protocols/$protocols"
  copy="$work/$name"
  mkdir -p "$work"
  cp -R "$src" "$copy"
  pictures="${OUT:+$OUT/$rel}"
  pictures="${pictures:-$work/out}"
  mkdir -p "$pictures"
  log="$work/log"

  echo "== $rel"
  if ! "$ROOT/scripts/drop-fetch-steps.sh" "$copy" "${ARC[@]}" > "$log" 2>&1; then
    fail "$rel" "$log"; protocols_failed=$((protocols_failed + 1)); continue
  fi
  notes "$log"
  # A database in this store, under arc's hash of the copy's path.
  if ! grep -rqsxF "$(cd "$copy" && pwd -P)" "$ARCFORM_DB_DIR"/*/protocol-path; then
    if ! ( cd "$copy" && "${ARC[@]}" run ) > "$log" 2>&1; then
      echo "FAIL $rel: arc run failed:"; tail -20 "$log" | sed 's/^/    /'
      protocols_failed=$((protocols_failed + 1)); continue
    fi
  fi

  for panel in "$copy"/panels/*.yaml; do
    [ -f "$panel" ] || continue
    file="panels/$(basename "$panel")"
    png="$pictures/$(basename "$panel" .yaml).png"
    if "$BF" export-png "$copy" "$file" --out "$png" > /dev/null 2> "$log" && magic "$png" 89504e47; then
      echo "ok $rel/$file"; notes "$log"; panels_ok=$((panels_ok + 1))
    else
      fail "$rel/$file" "$log"; panels_failed=$((panels_failed + 1))
    fi
  done

  if [ -f "$copy/report.md" ]; then
    pdf="$pictures/report.pdf"
    if "$BF" export-pdf "$copy" --out "$pdf" > /dev/null 2> "$log" && magic "$pdf" 25504446; then
      echo "ok $rel/report.md"; notes "$log"; pdfs_ok=$((pdfs_ok + 1))
    else
      fail "$rel/report.md" "$log"; pdfs_failed=$((pdfs_failed + 1))
    fi
  fi
done < <(find "$RUNS" -name arcform.yaml -type f -not -path '*/build/*' | LC_ALL=C sort)

echo "== $protocols Protocols ($protocols_failed could not be run); panels: $panels_ok ok, $panels_failed failed; PDFs: $pdfs_ok ok, $pdfs_failed failed"
[ "$protocols" -gt 0 ] || { echo "no Protocol (an arcform.yaml beside a panels/ folder) under $RUNS"; exit 1; }
[ $((protocols_failed + panels_failed + pdfs_failed)) -eq 0 ]
