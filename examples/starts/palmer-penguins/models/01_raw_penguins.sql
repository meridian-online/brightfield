CREATE OR REPLACE TABLE "raw_penguins" AS
SELECT * FROM read_csv('data/penguins.csv')
