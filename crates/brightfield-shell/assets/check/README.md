# Probe files for `--check-type-source`

`open-probe.parquet` is the input the sealed read-back opens to prove a packaged build can read a Parquet on a machine that has never run DuckDB. It holds the same four rows as the CSV that check writes inline, so the only difference between the two opens is the reader. It is not a dataset and nothing else reads it; the starter datasets can change without touching it.

It was written with the DuckDB CLI pinned at the time (v1.5.6):

```sh
target/duckdb-cli/duckdb -c "COPY (SELECT * FROM (VALUES
  ('alice.a00@example.com', 3), ('bob.b01@example.org', 10),
  ('carol.c02@example.net', 17), ('dan.d03@example.edu', 24))
  t(reply_to, messages)) TO 'crates/brightfield-shell/assets/check/open-probe.parquet' (FORMAT parquet)"
```
