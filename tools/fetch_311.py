#!/usr/bin/env python3
"""Fetch NYC 311 public-disorder complaints as a second justification-channel proxy for §5c.

311 is the classic "broken-windows / disorder-policing" demand signal. We deliberately do NOT use
all 21M 311 rows (which are dominated by housing-maintenance like HEAT/HOT WATER and infrastructure
like Street Condition — correlated with poverty/age of stock, not enforcement). We take the
PUBLIC-ORDER subset — the complaint types that map to the disorder-policing rationale that
surveillance placement invokes: noise, illegal parking / blocked driveway / abandoned vehicle, drug
activity, graffiti, public drinking / urination, panhandling, disorderly youth, homeless-assistance,
non-emergency police matters. Dataset erm2-nwe9, YTD 2026 (parallels the crime YTD window).

Out: data/snapshots/crime/nyc311_disorder_points.csv  (lat,lon)
"""
import csv, os, sys, urllib.parse, urllib.request

DATASET = "erm2-nwe9"
BASE = f"https://data.cityofnewyork.us/resource/{DATASET}.csv"
OUT = "data/snapshots/crime/nyc311_disorder_points.csv"
PAGE = 50000
SINCE = "2026-01-01"

DISORDER = [
    "Noise - Residential", "Noise - Street/Sidewalk", "Noise - Commercial",
    "Noise - Vehicle", "Noise", "Noise - Park", "Noise - House of Worship",
    "Illegal Parking", "Blocked Driveway", "Abandoned Vehicle",
    "Drug Activity", "Graffiti", "Panhandling", "Disorderly Youth",
    "Drinking", "Urinating in Public", "Non-Emergency Police Matter",
    "Homeless Person Assistance", "Homeless Encampment",
]


def where_clause() -> str:
    types = ",".join("'" + t.replace("'", "''") + "'" for t in DISORDER)
    return (f"created_date >= '{SINCE}' AND latitude IS NOT NULL "
            f"AND complaint_type in ({types})")


def fetch_page(offset: int):
    q = {"$select": "latitude,longitude", "$where": where_clause(),
         "$order": "unique_key", "$limit": PAGE, "$offset": offset}
    url = BASE + "?" + urllib.parse.urlencode(q, quote_via=urllib.parse.quote)
    with urllib.request.urlopen(url, timeout=180) as r:
        return list(csv.reader(line.decode("utf-8") for line in r))[1:]


def main() -> int:
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    n = 0
    with open(OUT, "w", newline="") as f:
        w = csv.writer(f); w.writerow(["lat", "lon"])
        offset = 0
        while True:
            rows = fetch_page(offset)
            if not rows:
                break
            for lat, lon in rows:
                try:
                    w.writerow([f"{float(lat):.6f}", f"{float(lon):.6f}"]); n += 1
                except ValueError:
                    continue
            print(f"  fetched {offset + len(rows):,} ...", file=sys.stderr)
            offset += PAGE
            if len(rows) < PAGE:
                break
    print(f"wrote {n:,} 311 disorder points -> {OUT}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
