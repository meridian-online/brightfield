# Per-capita CO2 since 1990: G7 countries, China and India

Question: how have per-capita CO2 emissions changed since 1990 in each G7 country (Canada, France, Germany, Italy, Japan, United Kingdom, United States), compared with China and India?

End state: a line chart of per-capita CO2 from 1990 to the latest year, one line per country (the G7 shown as seven countries, not one average), a table of 1990 vs latest values with absolute and percentage change, and a report.

Data: Our World in Data's CO2 dataset, https://raw.githubusercontent.com/owid/co2-data/master/owid-co2-data.csv, fetched into `data/owid-co2-data.csv`. Measure: `co2_per_capita` (tonnes of CO2 per person per year, territorial production-based emissions, fossil fuels and industry, excluding land-use change).

In scope: the nine countries, 1990 to the latest year with data. Out of scope: consumption-based emissions, land-use change, other greenhouse gases, the EU as a bloc.

Checks: each of the nine countries has a 1990 value and a latest-year value; one row per country per year; no aggregate regions included.

Readers: Hugh.
