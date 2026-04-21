CREATE OR REPLACE VIEW "athletes" AS SELECT * FROM read_parquet('data/athletes.parquet')
