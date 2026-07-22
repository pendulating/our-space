#!/usr/bin/env python3
"""Fetch 2020 Census block-group centroids + ACS block-group demographics for the five NYC
counties. Two subcommands:

  centroids  (default) — population-weighted BG centroids from the Census "Centers of Population
              by Block Group" file (CenPop2020_Mean_BG36.txt, NY). Public domain, no key. Each
              row is the population-weighted mean centre of a block group — the right residential
              anchor for surveillance-exposure sampling. → data/snapshots/census/bg_centroids_nyc.csv

  acs        — ACS 5-year (2022) block-group demographics for the disparity regressions +
              mode-choice calibration (plan §3b/§6): race/ethnicity (B03002), median household
              income (B19013), commute mode (B08301), tenure (B25003), poverty (B17001). Needs a
              Census API key in env `CENSUS_API_KEY` (auto-loaded from ./.env if present).
              → data/snapshots/census/acs_nyc.csv, keyed by 12-digit GEOID.

Both keyed by the 12-digit block-group GEOID — the join key for R_i / A_i. Stdlib only.
"""
import csv
import io
import json
import os
import sys
import urllib.parse
import urllib.request

NYC = {"005": "Bronx", "047": "Brooklyn", "061": "Manhattan", "081": "Queens", "085": "Staten Island"}
CENTROID_URL = "https://www2.census.gov/geo/docs/reference/cenpop2020/blkgrp/CenPop2020_Mean_BG36.txt"
ACS_YEAR = os.environ.get("ACS_YEAR", "2022")


def load_dotenv() -> None:
    """Populate os.environ from ./.env for KEY=VALUE lines (does not override real env)."""
    try:
        with open(".env") as f:
            for line in f:
                line = line.strip()
                if not line or line.startswith("#") or "=" not in line:
                    continue
                k, v = line.split("=", 1)
                os.environ.setdefault(k.strip(), v.strip().strip('"').strip("'"))
    except FileNotFoundError:
        pass


def fetch_centroids() -> int:
    out = "data/snapshots/census/bg_centroids_nyc.csv"
    print(f"fetching {CENTROID_URL} ...", file=sys.stderr)
    raw = urllib.request.urlopen(CENTROID_URL, timeout=120).read().decode("utf-8-sig")
    rows, per_county = [], {}
    for r in csv.DictReader(io.StringIO(raw)):
        cty = r["COUNTYFP"]
        if cty not in NYC or r["BLKGRPCE"] == "0":
            continue
        geoid = r["STATEFP"] + cty + r["TRACTCE"] + r["BLKGRPCE"]
        rows.append((geoid, float(r["LATITUDE"]), float(r["LONGITUDE"]), int(r["POPULATION"])))
        per_county[cty] = per_county.get(cty, 0) + 1
    rows.sort()
    os.makedirs(os.path.dirname(out), exist_ok=True)
    with open(out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["id", "lat", "lon", "population"])
        for geoid, lat, lon, pop in rows:
            w.writerow([geoid, f"{lat:.6f}", f"{lon:.6f}", pop])
    print(f"wrote {len(rows)} block groups ({sum(p for *_, p in rows):,} residents) -> {out}", file=sys.stderr)
    for cty, n in sorted(per_county.items()):
        print(f"  {cty} {NYC[cty]:14s}: {n} block groups", file=sys.stderr)
    return 0


# ACS 5-year variable → output column. Race/eth is B03002 (Hispanic-or-Latino-by-Race, the
# standard non-overlapping breakdown); the rest are headline income / mode / tenure / poverty.
ACS_VARS = {
    "B03002_001E": "pop_total",
    "B03002_003E": "white_nh",
    "B03002_004E": "black_nh",
    "B03002_006E": "asian_nh",
    "B03002_012E": "hispanic",
    "B19013_001E": "median_hh_income",
    "B08301_001E": "commute_total",
    "B08301_002E": "commute_car",       # car, truck, or van
    "B08301_010E": "commute_transit",   # public transit (excl. taxi)
    "B08301_011E": "commute_bus",       # bus (transit sub-mode: street-level capture)
    "B08301_012E": "commute_subway",    # subway or elevated rail (transit sub-mode)
    "B08301_019E": "commute_walk",
    "B08301_021E": "commute_wfh",       # worked from home
    "B25003_001E": "occ_units",
    "B25003_002E": "owner_occ",
    "B25003_003E": "renter_occ",
    "B17001_001E": "pov_universe",
    "B17001_002E": "below_poverty",
}
# Census "null" sentinels (median income unavailable / suppressed).
NULLS = {"-666666666", "-999999999", "-888888888", "", None}


def fetch_acs() -> int:
    load_dotenv()
    key = os.environ.get("CENSUS_API_KEY")
    if not key:
        print("ERROR: CENSUS_API_KEY not set (add it to ./.env or export it). Get a free key at "
              "https://api.census.gov/data/key_signup.html", file=sys.stderr)
        return 2

    varlist = ",".join(ACS_VARS)
    out = "data/snapshots/census/acs_nyc.csv"
    rows, per_county = {}, {}
    for cty in sorted(NYC):
        q = urllib.parse.urlencode({
            "get": varlist,
            "for": "block group:*",
            "in": f"state:36 county:{cty} tract:*",
            "key": key,
        })
        url = f"https://api.census.gov/data/{ACS_YEAR}/acs/acs5?{q}"
        print(f"  ACS {ACS_YEAR} county {cty} ({NYC[cty]}) ...", file=sys.stderr)
        data = json.load(urllib.request.urlopen(url, timeout=120))
        header, body = data[0], data[1:]
        idx = {name: i for i, name in enumerate(header)}
        for row in body:
            geoid = row[idx["state"]] + row[idx["county"]] + row[idx["tract"]] + row[idx["block group"]]
            rec = {}
            for var, col in ACS_VARS.items():
                v = row[idx[var]]
                rec[col] = "" if v in NULLS else v
            rows[geoid] = rec
            per_county[cty] = per_county.get(cty, 0) + 1

    os.makedirs(os.path.dirname(out), exist_ok=True)
    cols = ["id"] + list(ACS_VARS.values())
    with open(out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(cols)
        for geoid in sorted(rows):
            w.writerow([geoid] + [rows[geoid][c] for c in ACS_VARS.values()])
    print(f"wrote {len(rows)} block groups × {len(ACS_VARS)} ACS vars -> {out}", file=sys.stderr)
    for cty, n in sorted(per_county.items()):
        print(f"  {cty} {NYC[cty]:14s}: {n} block groups", file=sys.stderr)
    return 0


def main() -> int:
    mode = sys.argv[1] if len(sys.argv) > 1 else "centroids"
    if mode == "centroids":
        return fetch_centroids()
    if mode == "acs":
        return fetch_acs()
    if mode == "all":
        return fetch_centroids() or fetch_acs()
    print(f"usage: {sys.argv[0]} [centroids|acs|all]", file=sys.stderr)
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
