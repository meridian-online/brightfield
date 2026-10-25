#!/usr/bin/env bash
# Cut the reduced copies that the three fetched gallery starts are drawn from
# in the per-push tests: NYC TLC yellow taxi trips, UCI Online Retail II and
# Epoch AI's notable models.
#
#   scripts/cut-start-fixtures.sh [DUCKDB]
#       downloads each start's source from its issuer (about 110 MB in all),
#       writes crates/brightfield-shell/tests/data/<start>-reduced.parquet and
#       a provenance file <start>-reduced.json beside each, and prints what it
#       wrote. DUCKDB is the DuckDB shell to cut with,
#       `target/duckdb-cli/duckdb` when omitted (scripts/fetch-duckdb-cli.sh
#       puts the pinned one there). Online Retail's workbook is read with
#       DuckDB's `excel` extension, which the shell installs on first use.
#
# WHY COPIES AT ALL. Each of these starts reads its data from the issuer's own
# URL the first time it is opened, so nothing it draws is shipped in the
# binary. The thumbnails and the hermetic tests may not reach the network, so
# they run each start's Protocol over a committed copy instead
# (tests/support/shipped_protocol.rs, `boot_offline`). The copies are reduced:
# small enough to commit, and holding nothing the starts are not allowed to
# show.
#
# WHAT EACH KEEPS.
#
# - nyc-yellow-taxi-reduced.parquet stands in for the fetched month itself, so
#   the start's own load step and trips step run over it unchanged. It keeps
#   100,000 trips drawn at random (a fixed seed, one thread) from the
#   3,724,889 in the TLC's January 2026 file, and the five columns the trips
#   step reads: tpep_pickup_datetime, trip_distance, PULocationID,
#   passenger_count and total_amount. A uniform sample keeps the shape of every
#   view (the two weekly clocks, the airport hump, the record mix) at about a
#   thirty-seventh of the counts.
#
# - online-retail-ii-reduced.parquet stands in for the `retail_lines` table,
#   because the source is a zipped workbook that a hermetic test could only
#   read through an extension it would have to download. It is made by running
#   the start's own `retail_lines` step on UCI's workbook, then keeping one row
#   per purchase (an invoice that is not a cancellation and has a positive
#   value) with its lines summed into one: quantity 1 and price the invoice's
#   value. The `retail_orders` step makes the same rows from it as from the
#   full table, except that each order counts one line. It holds no Customer
#   ID: the step has already replaced it with each customer's first month and
#   the number of each of their orders. Product codes and descriptions are
#   left out, since nothing downstream reads them.
#
# - epoch-ai-models-reduced.parquet stands in for the `raw_models` table. It
#   is made by running the start's own `raw_models` step, which leaves out the
#   Authors and Abstract columns, and keeps every model and the eight columns
#   the `models` step reads: Model, Organization, Publication date, Training
#   compute (FLOP), Domain, Organization categorization, Model accessibility
#   and Confidence.
#
# Each provenance file names the issuer, the terms and the credit the copy is
# shared under, the URL it was cut from and when, the copy's rows and sha256,
# and what it keeps.
#
# WHEN TO RUN IT. When a start's Protocol changes the columns it reads, or a
# thumbnail should follow the issuer's newer data (Epoch AI's file grows as
# models are released; the other two are fixed). Then re-shoot the thumbnails
#   UPDATE_SNAPSHOTS=1 cargo test -p brightfield-shell --test front_door every_shipped_start
# and commit the copies, their provenance files and the PNGs together.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DUCKDB="${1:-$ROOT/target/duckdb-cli/duckdb}"
DATA="$ROOT/crates/brightfield-shell/tests/data"
STARTS="$ROOT/examples/starts"
CUT_ON="$(date -u +%Y-%m-%d)"

TAXI_URL="https://d37ci6vzurychx.cloudfront.net/trip-data/yellow_tripdata_2026-01.parquet"
RETAIL_URL="https://archive.ics.uci.edu/static/public/502/online+retail+ii.zip"
EPOCH_URL="https://epoch.ai/data/notable_ai_models.csv"

if [ ! -x "$DUCKDB" ]; then
  echo "no DuckDB shell at $DUCKDB: run scripts/fetch-duckdb-cli.sh or name one" >&2
  exit 1
fi
# A DuckDB shell that thinks an agent is driving it changes its output format.
unset AI_AGENT CLAUDECODE

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The SELECT of a step's model: its first line is the CREATE that names the
# table, which the cut makes itself.
step_select() {
  tail -n +2 "$1"
}

# provenance <file> <issuer> <terms> <terms url> <credit> <url> <copy> <keeps>
provenance() {
  local out="$1" issuer="$2" terms="$3" terms_url="$4" credit="$5" url="$6" copy="$7" keeps="$8"
  local rows sha
  rows=$("$DUCKDB" -noheader -list -c "SELECT count(*) FROM read_parquet('$copy')")
  sha=$(shasum -a 256 "$copy" | cut -d' ' -f1)
  ISSUER="$issuer" TERMS="$terms" TERMS_URL="$terms_url" CREDIT="$credit" URL="$url" \
    CUT_ON="$CUT_ON" COPY_NAME="$(basename "$copy")" ROWS="$rows" SHA="$sha" KEEPS="$keeps" \
    python3 -I -c '
import json, os
e = os.environ
print(json.dumps({
    "issuer": e["ISSUER"],
    "terms": {"title": e["TERMS"], "url": e["TERMS_URL"]},
    "credit": e["CREDIT"],
    "source": {"url": e["URL"], "cut_on": e["CUT_ON"]},
    "fixture": {"path": e["COPY_NAME"], "rows": int(e["ROWS"]), "sha256": e["SHA"], "keeps": e["KEEPS"]},
}, indent=2, ensure_ascii=False))
' >"$out"
  echo "wrote $copy: $rows rows, $(wc -c <"$copy" | tr -d ' ') bytes"
}

# --- NYC TLC yellow taxi, January 2026 ---------------------------------------
TAXI_SRC="$WORK/yellow_tripdata_2026-01.parquet"
TAXI_OUT="$DATA/nyc-yellow-taxi-reduced.parquet"
curl --fail --silent --show-error --location --output "$TAXI_SRC" "$TAXI_URL"
"$DUCKDB" -c "
SET threads = 1;
COPY (
  SELECT * FROM (
    SELECT tpep_pickup_datetime, trip_distance, PULocationID, passenger_count, total_amount
    FROM read_parquet('$TAXI_SRC')
    USING SAMPLE reservoir(100000 ROWS) REPEATABLE (2026)
  )
  ORDER BY tpep_pickup_datetime, PULocationID, trip_distance, total_amount, passenger_count
) TO '$TAXI_OUT' (FORMAT parquet, COMPRESSION zstd);
"
provenance "$DATA/nyc-yellow-taxi-reduced.json" \
  "NYC Taxi and Limousine Commission" \
  "Published as open data under the City of New York's terms of use" \
  "https://www.nyc.gov/site/tlc/about/tlc-trip-record-data.page" \
  "Data: NYC Taxi and Limousine Commission, TLC Trip Record Data" \
  "$TAXI_URL" "$TAXI_OUT" \
  "100,000 trips sampled at random (fixed seed) from the month's file, with the five columns the start's trips step reads"

# --- UCI Online Retail II -----------------------------------------------------
RETAIL_DIR="$WORK/online-retail-ii"
RETAIL_OUT="$DATA/online-retail-ii-reduced.parquet"
mkdir -p "$RETAIL_DIR/data"
curl --fail --silent --show-error --location --output "$RETAIL_DIR/data/online_retail_ii.zip" "$RETAIL_URL"
unzip -q -o "$RETAIL_DIR/data/online_retail_ii.zip" online_retail_II.xlsx -d "$RETAIL_DIR/data"
(
  cd "$RETAIL_DIR"
  "$DUCKDB" -c "
SET threads = 1;
INSTALL excel;
LOAD excel;
CREATE TABLE retail_lines AS $(step_select "$STARTS/online-retail-ii/models/01_retail_lines.sql");
COPY (
  SELECT invoice, 1 AS quantity, sum(quantity * price) AS price, min(invoice_time) AS invoice_time,
    any_value(country) AS country, bool_or(customer_id_recorded) AS customer_id_recorded,
    any_value(customer_first_month) AS customer_first_month,
    any_value(customer_order_number) AS customer_order_number
  FROM retail_lines
  WHERE invoice NOT LIKE 'C%'
  GROUP BY invoice
  HAVING sum(quantity * price) > 0
  ORDER BY invoice
) TO '$RETAIL_OUT' (FORMAT parquet, COMPRESSION zstd);
"
)
provenance "$DATA/online-retail-ii-reduced.json" \
  "UCI Machine Learning Repository" \
  "Creative Commons Attribution 4.0 International (CC BY 4.0)" \
  "https://creativecommons.org/licenses/by/4.0/" \
  "Chen, D. (2012). Online Retail II [Dataset]. UCI Machine Learning Repository. https://doi.org/10.24432/C5CG6D" \
  "$RETAIL_URL" "$RETAIL_OUT" \
  "One row per purchase, its lines summed into one, from the start's own retail_lines step; no Customer ID, product code or description"

# --- Epoch AI notable models --------------------------------------------------
EPOCH_DIR="$WORK/epoch-ai-models"
EPOCH_OUT="$DATA/epoch-ai-models-reduced.parquet"
mkdir -p "$EPOCH_DIR/data"
curl --fail --silent --show-error --location --output "$EPOCH_DIR/data/notable_ai_models.csv" "$EPOCH_URL"
(
  cd "$EPOCH_DIR"
  "$DUCKDB" -c "
SET threads = 1;
CREATE TABLE raw_models AS $(step_select "$STARTS/epoch-ai-models/models/01_raw_models.sql");
COPY (
  SELECT \"Model\", \"Organization\", \"Publication date\", \"Training compute (FLOP)\", \"Domain\",
    \"Organization categorization\", \"Model accessibility\", \"Confidence\"
  FROM raw_models
  ORDER BY \"Publication date\", \"Model\"
) TO '$EPOCH_OUT' (FORMAT parquet, COMPRESSION zstd);
"
)
provenance "$DATA/epoch-ai-models-reduced.json" \
  "Epoch AI" \
  "Creative Commons Attribution 4.0 International (CC BY 4.0)" \
  "https://creativecommons.org/licenses/by/4.0/" \
  "Epoch AI, 'Data on AI models'. Published online at epoch.ai. Retrieved from https://epoch.ai/data/ai-models" \
  "$EPOCH_URL" "$EPOCH_OUT" \
  "Every model, with the eight columns the start's models step reads, from its own raw_models step; no Authors or Abstract"
