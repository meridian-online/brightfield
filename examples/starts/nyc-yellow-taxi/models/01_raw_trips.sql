CREATE OR REPLACE TABLE "raw_trips" AS
SELECT * FROM read_parquet('data/yellow_tripdata_2026-01.parquet')
