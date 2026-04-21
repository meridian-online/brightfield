CREATE OR REPLACE VIEW "penguins" AS SELECT * FROM read_parquet('data/penguins.parquet')
