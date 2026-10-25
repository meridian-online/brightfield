CREATE OR REPLACE TABLE "raw_models" AS
SELECT * EXCLUDE ("Authors", "Abstract") FROM read_csv('data/notable_ai_models.csv')
