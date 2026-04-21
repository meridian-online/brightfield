CREATE OR REPLACE VIEW "weather" AS SELECT * FROM read_parquet('data/seattle-weather.parquet')
