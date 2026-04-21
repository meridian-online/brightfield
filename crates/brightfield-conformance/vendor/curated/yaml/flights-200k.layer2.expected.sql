CREATE OR REPLACE VIEW "flights" AS SELECT * FROM read_parquet('data/flights-200k.parquet')
