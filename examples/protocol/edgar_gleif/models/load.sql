-- Load the fetched sources into typed tables and normalise the join keys.
-- Run by the `load` step (depends on all fetches). CIK representations differ across
-- sources (EDGAR unpadded, GLEIF zero-padded) — normalise all to unpadded.

-- The SEC entity universe (the resolution left side + name/ticker enrichment source):
-- ~1.05M named filers incl. former names, tickers where present. cik is already
-- unpadded VARCHAR from the build. Multiple rows per CIK (former names) are collapsed
-- downstream — package.sql dedups EDGAR to one row per CIK, tier.sql dedups edges.
CREATE OR REPLACE TABLE edgar AS
  SELECT cik, company_name, ticker, exchange
  FROM read_parquet('build/sec_entities.parquet');

CREATE OR REPLACE TABLE gleif AS
  SELECT upper(trim(lei)) AS lei, legal_name, country, city, jurisdiction,
         registration_status AS reg_status
  FROM read_parquet('build/gleif.parquet');

-- Deterministic authoritative pairs — GLEIF SEC registrations. Type the SEC
-- identifier by its scheme (SEC-defined: S…=fund series, C…=share class, digits=CIK);
-- unpad CIKs to match EDGAR, keep series/class verbatim. `category` is GLEIF's own
-- authoritative entity type (FUND | GENERAL | BRANCH | …).
CREATE OR REPLACE TABLE gleif_ra_sec AS
  SELECT
    upper(trim(lei)) AS lei,
    CASE WHEN registered_as LIKE 'S%' THEN 'series'
         WHEN registered_as LIKE 'C%' THEN 'class'
         WHEN regexp_full_match(registered_as, '[0-9]+') THEN 'cik'
         ELSE 'other' END AS key_type,
    CASE WHEN regexp_full_match(registered_as, '[0-9]+')
         THEN CAST(registered_as AS BIGINT)::VARCHAR
         ELSE trim(registered_as) END AS key,
    category
  FROM read_csv('build/gleif_ra_sec.csv', header=true, all_varchar=true)
  WHERE lei IS NOT NULL AND registered_as IS NOT NULL AND registered_as <> ''
    -- Keep only the three EDGAR identifier schemes. RA000665 also carries a handful
    -- of SEC investment-adviser file numbers (`805-…`, Form ADV) — a different scheme
    -- that maps to no EDGAR CIK/series/class, so they'd be orphan edges. Drop them.
    AND (registered_as LIKE 'S%' OR registered_as LIKE 'C%'
         OR regexp_full_match(registered_as, '[0-9]+'));

-- SEC Form N-CEN (the annual fund filing itself — most direct authority). Glob over
-- the fetched quarters; union_by_name is robust to column-order drift across releases.
-- REGISTRANT.tsv: registrant CIK ↔ LEI. FUND_REPORTED_INFO.tsv: series SERIES_ID ↔ LEI.
CREATE OR REPLACE TABLE ncen_registrant AS
  SELECT DISTINCT CAST(CIK AS BIGINT)::VARCHAR AS key, upper(trim(LEI)) AS lei
  FROM read_csv('build/ncen/*/REGISTRANT.tsv', delim='\t', header=true, all_varchar=true, union_by_name=true)
  WHERE LEI IS NOT NULL AND LEI <> '' AND CIK IS NOT NULL;
CREATE OR REPLACE TABLE ncen_series AS
  SELECT DISTINCT trim(SERIES_ID) AS key, upper(trim(LEI)) AS lei
  FROM read_csv('build/ncen/*/FUND_REPORTED_INFO.tsv', delim='\t', header=true, all_varchar=true, union_by_name=true)
  WHERE LEI IS NOT NULL AND LEI <> '' AND SERIES_ID IS NOT NULL AND SERIES_ID <> '';
