CREATE OR REPLACE TABLE "raw_earthquakes" AS
SELECT * FROM read_parquet('data/earthquakes_2025.parquet')
