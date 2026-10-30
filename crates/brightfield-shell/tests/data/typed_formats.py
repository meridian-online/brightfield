#!/usr/bin/env python3
"""Write typed_formats.csv beside this script: 200 rows of columns written the way analysts write
them (sterling amounts with accounting negatives, dollar prices, percentages, rates, years, counts,
ISO dates and free text), so a flat open has something to cast.

Every value is arithmetic on the row number, so running this again writes the same bytes.
"""

import csv
import datetime
import pathlib

ROWS = 200
YEARS = [y for y in range(2001, 2025) if y != 2009]
PLACES = ["north", "south", "east", "west"]


def pounds(pence: int) -> str:
    """`pence` written as sterling, with a negative in parentheses."""
    whole, part = divmod(abs(pence), 100)
    text = f"£{whole:,}.{part:02d}"
    return f"({text})" if pence < 0 else text


def row(i: int) -> list[str]:
    if i % 40 == 7:
        revenue = pounds(-1200)
    else:
        # From about £0.2bn at the first row to about £3.1bn at the last.
        revenue = pounds(20_000_000_000 + i * 1_457_286_432 + (i * 37) % 100)
    price = 1250 + (i * 96750) // (ROWS - 1)
    return [
        revenue,
        f"${price // 100:,}.{price % 100:02d}",
        f"{12 + (i * 66) // (ROWS - 1)}%",
        f"{4.1 + (i % 6) / 10:.1f}%",
        str(YEARS[i % len(YEARS)]),
        str(i * i * 101),
        (datetime.date(2024, 1, 1) + datetime.timedelta(days=i)).isoformat(),
        f"note {i} from the {PLACES[i % 4]} office",
    ]


def main() -> None:
    out = pathlib.Path(__file__).with_name("typed_formats.csv")
    with out.open("w", newline="", encoding="utf-8") as f:
        w = csv.writer(f, lineterminator="\n")
        w.writerow(
            ["revenue", "price_usd", "share", "yield", "year", "downloads", "day", "notes"]
        )
        for i in range(ROWS):
            w.writerow(row(i))


if __name__ == "__main__":
    main()
