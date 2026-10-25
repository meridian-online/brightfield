CREATE OR REPLACE TABLE "earthquakes" AS
SELECT time, CAST(date_trunc('week', time) AS DATE) AS week, latitude, longitude, depth, mag, magType, CASE WHEN magType = 'mb' THEN 'body wave (mb)' WHEN magType LIKE 'mw%' THEN 'moment (Mw)' ELSE 'local (ML)' END AS magnitude_scale, place FROM raw_earthquakes
