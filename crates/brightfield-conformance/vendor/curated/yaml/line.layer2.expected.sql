CREATE OR REPLACE VIEW "aapl" AS SELECT * FROM read_parquet('data/stocks.parquet')
