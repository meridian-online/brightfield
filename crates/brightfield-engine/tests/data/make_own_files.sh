#!/bin/sh
# Regenerates own_files/sales.parquet, the one binary file in the folder fixture: a Parquet
# whose stem (`sales`) matches `sales.csv`, so two files in the folder want the same table name.
# The other files are checked in as plain text, apart from broken.csv, which holds bytes that
# are not valid text so DuckDB's CSV sniffer refuses it.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
duckdb -no-agent -c "COPY (SELECT 7 AS id, 'z' AS name) TO '$here/own_files/sales.parquet' (FORMAT parquet)"
