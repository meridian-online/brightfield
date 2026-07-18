-- Enrich every edge from the sources (by LEI → GLEIF; by CIK → EDGAR), stamp the
-- as_of, build a search corpus, and MATERIALISE the terminal table the `parquet_export`
-- operator writes to build/edgar_gleif.parquet (the Parquet the out-of-repo publish
-- seam consumes). Funds (key_type=series) don't join to EDGAR, so their display name
-- comes from GLEIF's legal_name via coalesce.
--
-- GRAIN: one row per (key_type, key, lei) edge. A CIK can carry several tickers, so
-- EDGAR is deduped to one row per CIK with the tickers aggregated (a comma list) —
-- otherwise the join fans an edge out into one row per ticker. (Ticker-grain vs
-- CIK-grain is a deliberate modelling choice; see README.)
--
-- Why a TABLE (not a COPY, not a VIEW): the terminal write is now the typed
-- `parquet_export` operator (arcform), so this step just builds the row set and the
-- operator owns the file — the export becomes a first-class produced asset instead of
-- a COPY the engine can't parse (which was a graph ISLAND that could silently ship a
-- stale Parquet on a green run). A TABLE, not a VIEW, because `as_of` reads
-- `getenv('ARC_PARAM_AS_OF')`, which must be evaluated HERE (this step's env) — a lazy
-- view would defer it to the export step's context and stamp NULL. The row ORDER is
-- applied by `parquet_export`'s `order_by` (a total order — see arcform.yaml).
CREATE OR REPLACE TABLE edgar_gleif_out AS
  SELECT
    x.key_type,
    x.key,
    e.ticker,
    coalesce(e.company_name, g.legal_name) AS company_name,
    x.lei,
    g.legal_name,
    coalesce(x.category, 'GENERAL')        AS category,
    g.jurisdiction,
    g.reg_status,
    x.method,
    x.tier,
    round(x.confidence, 6)                 AS confidence,
    CAST(nullif(getenv('ARC_PARAM_AS_OF'), '') AS DATE) AS as_of,
    lower(strip_accents(concat_ws(' ',
      coalesce(e.company_name, g.legal_name), g.legal_name, e.ticker, x.lei))) AS corpus
  FROM crosswalk_edges x
  LEFT JOIN gleif g ON g.lei = x.lei
  LEFT JOIN (
    SELECT cik,
           string_agg(DISTINCT ticker, ',' ORDER BY ticker) AS ticker,
           -- A CIK carries several name variants (former names). `min` picks one
           -- DETERMINISTICALLY; `any_value` returned an arbitrary variant that varied
           -- run-to-run (DuckDB parallel scan), making both the displayed name AND the
           -- export's sort key non-reproducible. (The row SET is unchanged either way.)
           min(company_name)                                AS company_name
    FROM edgar GROUP BY cik
  ) e ON x.key_type = 'cik' AND e.cik = x.key;
