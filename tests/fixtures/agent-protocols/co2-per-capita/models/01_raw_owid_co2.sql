CREATE OR REPLACE TABLE "raw_owid_co2" AS
SELECT * FROM read_csv('data/owid-co2-data.csv')
