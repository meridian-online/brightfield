#!/usr/bin/env bash
# Cut the reduced copy of the published EDGAR ↔ GLEIF crosswalk that the
# crosswalk chart start renders from in the per-push thumbnail tests.
#
#   scripts/cut-crosswalk-fixture.sh [DUCKDB]
#       downloads the published Parquet, writes the reduced copy and its
#       provenance file over the committed ones, and prints what it recorded.
#       DUCKDB is the DuckDB shell to cut with, `target/duckdb-cli/duckdb` when
#       omitted (scripts/fetch-duckdb-cli.sh puts the pinned one there).
#
# WHY A FIXTURE AT ALL. The crosswalk chart (examples/remote/edgar-gleif-crosswalk.yaml)
# reads a 10 MB Parquet over https, so its two front-door thumbnails could only
# be redrawn by the nightly network tests. Every pull request that moved the
# interface re-shot the other starts' thumbnails and left these two behind,
# and they drifted until the nightly went red. With this copy on disk the
# per-push thumbnail tests redraw them with no network, like every other start.
#
# WHAT IT KEEPS. The chart reads three columns and nothing else: `tier` and
# `method` as they are, and `corpus` only through `length(corpus)`. So the copy
# keeps `tier` and `method` and replaces `corpus` with a run of `x` of the same
# length, in the published row order. Every bin and every count the chart draws
# is the same as over the published table, and none of the published text is
# copied into this repository. It is about 240 KB against 10 MB.
#
# WHEN TO RUN IT. When the nightly's check that the published table is still
# the one this copy was cut from goes red: the provenance file records the
# published file's size, sha256 and row count, and the check compares them.
# Then re-shoot the crosswalk chart's thumbnails offline with
#   UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test front_door every_shipped_start
# and commit the copy, its provenance file and the PNGs together.
set -euo pipefail

URL="https://openlake.meridian.online/edgar_gleif.parquet"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DUCKDB="${1:-$ROOT/target/duckdb-cli/duckdb}"
DATA="$ROOT/crates/brightfield-shell/tests/data"
FIXTURE="$DATA/edgar_gleif_reduced.parquet"
PROVENANCE="$DATA/edgar_gleif_reduced.json"

if [ ! -x "$DUCKDB" ]; then
  echo "no DuckDB shell at $DUCKDB: run scripts/fetch-duckdb-cli.sh or name one" >&2
  exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
SOURCE="$WORK/edgar_gleif.parquet"

curl --fail --silent --show-error --location --output "$SOURCE" "$URL"

source_bytes=$(wc -c <"$SOURCE" | tr -d ' ')
source_sha=$(shasum -a 256 "$SOURCE" | cut -d' ' -f1)
source_rows=$("$DUCKDB" -noheader -list -c "SELECT count(*) FROM read_parquet('$SOURCE')")

# One thread, so the row groups are written in the published order and two
# cuts of the same source are the same bytes.
"$DUCKDB" -c "
SET threads = 1;
COPY (
  SELECT tier, method, repeat('x', length(corpus)) AS corpus
  FROM read_parquet('$SOURCE')
) TO '$FIXTURE' (FORMAT parquet, COMPRESSION zstd);
"

fixture_rows=$("$DUCKDB" -noheader -list -c "SELECT count(*) FROM read_parquet('$FIXTURE')")
if [ "$fixture_rows" != "$source_rows" ]; then
  echo "the copy has $fixture_rows rows and the published table $source_rows" >&2
  exit 1
fi
fixture_sha=$(shasum -a 256 "$FIXTURE" | cut -d' ' -f1)

cat >"$PROVENANCE" <<JSON
{
  "source": {
    "url": "$URL",
    "bytes": $source_bytes,
    "sha256": "$source_sha",
    "rows": $source_rows
  },
  "fixture": {
    "path": "edgar_gleif_reduced.parquet",
    "sha256": "$fixture_sha",
    "columns": ["tier", "method", "corpus"],
    "corpus": "a run of x as long as the published corpus value"
  }
}
JSON

cat "$PROVENANCE"
