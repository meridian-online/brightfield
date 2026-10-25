CREATE OR REPLACE TABLE "measures" AS
SELECT * FROM read_csv('data/measures.csv')
