#!/usr/bin/env python3
"""Fetch LODES8 home→work commute OD for NYC and aggregate census-block flows to block groups.

Source: U.S. Census LEHD **LODES8** Origin-Destination, New York state, main part
(`ny_od_main_JT00_<year>` = jobs where home *and* work are in NY), public domain. LODES8 is on
2020 census blocks, so it lines up with the 2020 TIGER / CenPop centroids used for R_i. We keep
flows whose home *and* work block are in the five NYC counties (both endpoints routable on the
citywide drive graph), aggregate the 15-digit block geocodes to 12-digit block-group GEOIDs, and
sum total jobs (S000) and low-wage jobs (SE01, ≤ $1,250/mo). Output is the OD backbone for the
plan's activity-space exposure A_i (`batch od-exposure`).

Requires `duckdb` on PATH (as the taxi bake does) for the 7.5M-row aggregation.

Out:
  data/snapshots/lodes/ny_od_main_JT00_<year>.csv.gz   (raw)
  data/snapshots/lodes/bg_od_nyc.csv                   (home_bg,work_bg,jobs,low_wage)
"""
import os
import subprocess
import sys
import urllib.request

YEAR = os.environ.get("LODES_YEAR", "2022")
BASE = "https://lehd.ces.census.gov/data/lodes/LODES8/ny/od"
DEST = "data/snapshots/lodes"
NYC = ("36005", "36047", "36061", "36081", "36085")  # Bronx, Kings, NY, Queens, Richmond


def fetch(name: str) -> str:
    path = os.path.join(DEST, name)
    if os.path.exists(path):
        print(f"  have {name}", file=sys.stderr)
        return path
    url = f"{BASE}/{name}"
    print(f"  fetching {url} ...", file=sys.stderr)
    urllib.request.urlretrieve(url, path)
    return path


def main() -> int:
    os.makedirs(DEST, exist_ok=True)
    main_gz = fetch(f"ny_od_main_JT00_{YEAR}.csv.gz")
    fetch(f"ny_od_aux_JT00_{YEAR}.csv.gz")  # workers living out-of-state; kept for later inflow work

    counties = ",".join(f"'{c}'" for c in NYC)
    out = os.path.join(DEST, "bg_od_nyc.csv")
    sql = f"""
    COPY (
      SELECT substr(h_geocode::VARCHAR,1,12) AS home_bg,
             substr(w_geocode::VARCHAR,1,12) AS work_bg,
             SUM(S000)::INT AS jobs,
             SUM(SE01)::INT AS low_wage
      FROM read_csv_auto('{main_gz}', header=true)
      WHERE substr(h_geocode::VARCHAR,1,5) IN ({counties})
        AND substr(w_geocode::VARCHAR,1,5) IN ({counties})
      GROUP BY 1,2
    ) TO '{out}' (HEADER, DELIMITER ',');
    """
    print("  aggregating block→BG via duckdb ...", file=sys.stderr)
    subprocess.run(["duckdb", "-c", sql], check=True)

    n = sum(1 for _ in open(out)) - 1
    print(f"wrote {n} NYC→NYC block-group OD pairs -> {out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
