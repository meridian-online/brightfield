#!/usr/bin/env bash
# Cut the year of earthquakes the USGS earthquakes start bundles, from the
# USGS's own event service.
#
#   scripts/cut-usgs-earthquakes.sh [DUCKDB]
#       downloads every event of magnitude 4.5 and up in 2025 as CSV, keeps the
#       rows and columns below, writes them as zstd Parquet into the start's
#       Protocol folder, and checks the row count and the size.
#       DUCKDB is the DuckDB shell to cut with, `target/duckdb-cli/duckdb` when
#       omitted (scripts/fetch-duckdb-cli.sh puts the pinned one there).
#
# WHY A BUNDLED FILE. The start opens with no network, so its data sits in the
# Protocol folder and nothing is fetched. The USGS's event data is a work of
# the US federal government and in the public domain, which permits shipping
# it, and a year of it is small.
#
# WHAT IT KEEPS. Only events the USGS's own network reported (`net = 'us'`) of
# type `earthquake`. The service also returns events located by other networks
# that contribute to the catalogue, and those networks' terms are their own;
# keeping the USGS's own rows keeps the public-domain claim exact. Quarry
# blasts and other event types are dropped because the start is about
# earthquakes. Seven columns are kept: time, latitude, longitude, depth, mag,
# magType and place. The identifiers, the networks' bookkeeping and the error
# estimates are left out, since the dashboard reads none of them.
#
# WHEN TO RUN IT. Once, to make the file; again only if the USGS revises 2025's
# catalogue and the start should follow it. The USGS does revise events after
# the fact, so a later run can differ by a few rows, and the counts below are
# what to update with a deliberate re-cut. Then raise the start's Protocol
# revision in crates/brightfield-shell/src/starts.rs, re-shoot its thumbnails
# with
#   UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test front_door every_shipped_start
# and commit the file and the PNGs together.
set -euo pipefail

URL="https://earthquake.usgs.gov/fdsnws/event/1/query?format=csv&starttime=2025-01-01&endtime=2026-01-01&minmagnitude=4.5"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DUCKDB="${1:-$ROOT/target/duckdb-cli/duckdb}"
OUT="$ROOT/examples/starts/usgs-earthquakes-2025/data/earthquakes_2025.parquet"
EXPECTED_ROWS=8502
# About 211 KB; the bounds allow for a different DuckDB writing the same rows.
MIN_BYTES=180000
MAX_BYTES=250000

if [ ! -x "$DUCKDB" ]; then
  echo "no DuckDB shell at $DUCKDB: run scripts/fetch-duckdb-cli.sh or name one" >&2
  exit 1
fi
# A DuckDB shell that thinks an agent is driving it changes its output format.
unset AI_AGENT CLAUDECODE

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SOURCE="$WORK/query.csv"

curl --fail --silent --show-error --location --output "$SOURCE" "$URL"
mkdir -p "$(dirname "$OUT")"

# One thread and an explicit order, so two cuts of the same source are the
# same bytes. `time` arrives as an ISO timestamp in UTC and is kept as one.
"$DUCKDB" -c "
SET threads = 1;
SET TimeZone = 'UTC';
COPY (
  SELECT
    CAST(time AS TIMESTAMP) AS time,
    latitude, longitude, depth, mag, magType, place
  FROM read_csv('$SOURCE')
  WHERE net = 'us' AND type = 'earthquake'
  ORDER BY time, latitude, longitude
) TO '$OUT' (FORMAT parquet, COMPRESSION zstd);
"

rows=$("$DUCKDB" -noheader -list -c "SELECT count(*) FROM read_parquet('$OUT')")
bytes=$(wc -c <"$OUT" | tr -d ' ')
echo "wrote $OUT: $rows rows, $bytes bytes"

if [ "$rows" != "$EXPECTED_ROWS" ]; then
  echo "expected $EXPECTED_ROWS rows and the cut has $rows" >&2
  exit 1
fi
if [ "$bytes" -lt "$MIN_BYTES" ] || [ "$bytes" -gt "$MAX_BYTES" ]; then
  echo "expected about 211 KB and the cut is $bytes bytes" >&2
  exit 1
fi
