#!/usr/bin/env python3
"""Fetch NYPD complaint (crime) points for the §5c crime-control ladder.

The crime-justification channel: placement "justified by" crime/enforcement data risks laundering
a racialized disparity, so we control for crime density as a mediator. We use the NYPD Complaint
Data Current (Year-To-Date), dataset 5uac-w243 — every complaint carries a law category
(FELONY/MISDEMEANOR/VIOLATION) and a geocode. Spatial distribution of crime is stable year-to-year,
so a YTD slice is an adequate spatial covariate. We keep only lat/lon + law category (felony flag
lets us report a felony-only robustness).

Out: data/snapshots/crime/nypd_points.csv  (lat,lon,felony)
"""
import csv
import os
import sys
import urllib.parse
import urllib.request

DATASET = "5uac-w243"  # NYPD Complaint Data Current (YTD)
BASE = f"https://data.cityofnewyork.us/resource/{DATASET}.csv"
DEST = "data/snapshots/crime"
OUT = os.path.join(DEST, "nypd_points.csv")
PAGE = 50000


def fetch_page(offset: int) -> list[list[str]]:
    q = {
        "$select": "latitude,longitude,law_cat_cd",
        "$where": "latitude IS NOT NULL AND longitude IS NOT NULL",
        "$order": "cmplnt_num",  # stable pagination order
        "$limit": PAGE,
        "$offset": offset,
    }
    url = BASE + "?" + urllib.parse.urlencode(q, quote_via=urllib.parse.quote)
    with urllib.request.urlopen(url, timeout=120) as r:
        rows = list(csv.reader(line.decode("utf-8") for line in r))
    return rows[1:]  # drop header


def main() -> int:
    os.makedirs(DEST, exist_ok=True)
    n = 0
    with open(OUT, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["lat", "lon", "felony"])
        offset = 0
        while True:
            rows = fetch_page(offset)
            if not rows:
                break
            for lat, lon, cat in rows:
                try:
                    la, lo = float(lat), float(lon)
                except ValueError:
                    continue
                w.writerow([f"{la:.6f}", f"{lo:.6f}", 1 if cat == "FELONY" else 0])
                n += 1
            print(f"  fetched {offset + len(rows):,} rows ...", file=sys.stderr)
            offset += PAGE
            if len(rows) < PAGE:
                break
    print(f"wrote {n:,} crime points -> {OUT}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
