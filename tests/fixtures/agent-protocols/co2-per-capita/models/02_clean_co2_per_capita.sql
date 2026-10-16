CREATE OR REPLACE TABLE "clean_co2_per_capita" AS
SELECT country, iso_code, CAST(year AS SMALLINT) AS year, CASE WHEN iso_code IN ('CHN','IND') THEN country ELSE 'G7' END AS grouping, population, co2 AS co2_mt, co2_per_capita AS co2_t_per_person FROM raw_owid_co2 WHERE iso_code IN ('CAN','FRA','DEU','ITA','JPN','GBR','USA','CHN','IND') AND year >= 1990 AND co2_per_capita IS NOT NULL
