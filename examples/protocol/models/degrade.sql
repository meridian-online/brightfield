-- Degrade fixture. The MIDDLE statement below is
-- deliberately not SQL: it tokenises (plain words) but fails to parse, so it
-- must become an opaque chip while both siblings still explode into typed
-- nodes.

CREATE OR REPLACE TABLE staged AS
  SELECT * FROM read_csv('build/raw.csv', header=true);

SELEC every FORM here IS deliberately unparseable;

CREATE OR REPLACE TABLE cleaned AS
  SELECT * FROM staged WHERE x IS NOT NULL;
