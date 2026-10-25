CREATE OR REPLACE TABLE "earthquakes" AS
SELECT time, CAST(date_trunc('week', time) AS DATE) AS week, latitude, longitude, depth, mag, magType, CASE WHEN magType = 'mb' THEN 'body wave (mb)' WHEN magType LIKE 'mw%' THEN 'moment (Mw)' WHEN magType LIKE 'ml%' THEN 'local (ML)' WHEN magType LIKE 'ms%' THEN 'surface wave (Ms)' ELSE magType END AS magnitude_scale, place FROM raw_earthquakes
