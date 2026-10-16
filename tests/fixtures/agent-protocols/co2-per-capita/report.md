Every G7 country emits less CO2 per person than it did in 1990, while China's per-capita emissions have roughly quadrupled and India's have more than tripled (`co2_per_capita_change`). The UK fell furthest, from 10.49 to 4.53 tonnes per person (-56.9%), followed by Germany (-48.8%) and France (-42.7%). Japan (-17.0%) and Canada (-18.6%) fell least. China rose from 2.15 to 8.66 tonnes (+302.1%) and India from 0.67 to 2.20 tonnes (+229.5%), and both reached their highest level in 2024, the latest year in the data.

Despite those falls, the US (14.20 t) and Canada (13.42 t) still emit more per person than China, and every G7 country still emits more per person than India. China passed France in 2010, Italy in 2012 and the UK in 2014, and has stayed above all three since 2020 (`china_overtakes_g7`). It was first above Germany in 2022 and Japan in 2023, but has not stayed above them every year since 2020. It has never been above the US or Canada in this period.

Most G7 countries peaked early. Germany's highest value is in 1990 itself, the UK's and France's in 1991, and the US's and Canada's in 2000. Italy peaked in 2004 and Japan in 2013.

Caveats:

- The measure is OWID's `co2_per_capita`: territorial (production-based) CO2 from fossil fuels and industry, excluding land-use change and other greenhouse gases. Emissions embodied in imports count against the producing country, so this measure flatters net importers such as the UK and the US relative to China. The OWID file also carries `consumption_co2_per_capita` if you want to see how much that changes the picture.
- 2024 is the latest year, and the most recent year's values are usually preliminary and may be revised.
- The fetch reads OWID's `master` branch with no pinned checksum, so re-running the Protocol later may pick up revised figures. These numbers come from the file as fetched on 2026-10-17.
- Data quality was good. All nine countries have a value in every year from 1990 to 2024 (315 rows, 9 × 35). FineType mistyped 28 mostly-empty numeric columns (as version strings, network ranges and dates). I corrected them by hand in `schemas/raw_owid_co2.json`, after which all 50,411 rows validated. None of those columns are used here.
- Countries are matched on ISO code (CAN, FRA, DEU, ITA, JPN, GBR, USA, CHN, IND), so OWID's regional aggregates cannot slip in.

Next steps: add consumption-based per-capita CO2 alongside the territorial figure, since that is the comparison most likely to narrow the gap between the G7 and China. Pin the fetch with a `sha256` if you want this exact snapshot to be reproducible.

## How this was derived

1. `fetch_owid_co2`: Download Our World in Data's CO2 dataset (skipped), made `data/owid-co2-data.csv`
2. `raw_owid_co2`: Load the OWID CO2 file as it arrives (skipped), made `raw_owid_co2` (50411 rows)
3. `clean_co2_per_capita`: Keep the seven G7 countries, China and India from 1990 on, with per-capita CO2 in tonnes per person; matched on ISO code so regional aggregates cannot slip in (skipped), made `clean_co2_per_capita` (315 rows)
4. `co2_per_capita_change`: One row per country: per-capita CO2 in 1990 and the latest year, the change in tonnes and percent, and the peak year since 1990 (skipped), made `co2_per_capita_change` (9 rows)
5. `china_overtakes_g7`: For each G7 country, the first year China's per-capita CO2 exceeded it, and whether it has stayed above since 2020 (success), made `china_overtakes_g7` (7 rows)

## Charts

Findings are checked against the Protocol's current database, not the run's.

- `panels/co2_per_capita_change_pct.yaml`: Change in per-capita CO2, 1990 to 2024 (percent)
- `panels/co2_per_capita_since_1990.yaml`: Per-capita CO2 since 1990, G7 countries vs China and India (tonnes per person)

## Run

- Run: `20261016-211510-2a03e7b8`
- Finished: 2026-10-16T21:15:10Z
- Outcome: success
- Assets, by content hash:
  - `table:china_overtakes_g7`: 4922e56afe92c6acace11ca5305fd72e4e5c86eb6418d2faabb6cd8d58fe2f92
  - `table:clean_co2_per_capita`: 2b7a89644d8e3c63a0f0641a3f8f9bab4a065e801eebdf5fa088805d5dc98c4a
  - `table:co2_per_capita_change`: 96898d6f03507bb80664a502414305501f4e2d70c8d4be44d82d7666350073a7
  - `file:data/owid-co2-data.csv`: 7f78e2b218ce4bb8c538bbec04fdc9a7982e8d40bff972e650df603899edd5f6
  - `table:raw_owid_co2`: 9f932a5898e94dc6fdfe73d9501bb5bdf4f736c03a290c266894d17a9aa65b2a
