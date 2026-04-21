CREATE OR REPLACE VIEW "walk" AS SELECT * FROM read_parquet('data/random-walk.parquet')
