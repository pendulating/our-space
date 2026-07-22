#!/usr/bin/env python3
"""Reproducible fetchers for every dataset under data/snapshots/.

Each snapshot directory holds a point-in-time copy of an upstream source; this
script can regenerate any of them. One subcommand per dataset:

    python3 tools/fetch_snapshots.py --list          # every dataset + its files
    python3 tools/fetch_snapshots.py amnesty dot     # fetch specific datasets
    python3 tools/fetch_snapshots.py all             # everything light (skips --heavy)
    python3 tools/fetch_snapshots.py all --heavy     # include multi-GB downloads
    python3 tools/fetch_snapshots.py tlc --force     # refetch even if present

Fetches are idempotent: a dataset whose files all exist is skipped unless
`--force`. Datasets with an existing dedicated script (fetch_census.py,
fetch_crime.py, fetch_311.py, fetch_enforcement.py, fetch_gtfs.py,
fetch_lodes.py, extract_landmarks.py) are delegated to it, so the fetch logic
lives in exactly one place.

NOTE: snapshots are pinned inputs, not live mirrors. Refetching pulls whatever
the upstream serves *today* — counts baked into the app (e.g. the ACE
violations constants in app-interactive/src/main.rs) may need re-transcribing
after a refresh. Provenance for every dataset: data/snapshots/README.md.

To stage a refetch without touching the pinned snapshots, set
OURSPACE_SNAPSHOT_DIR to a scratch directory and diff before promoting.
(Delegated scripts — fetch_census.py, fetch_gtfs.py, etc. — write to the repo's
data/snapshots/ regardless; only the fetchers in this file honor the override.)
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import os
import shutil
import subprocess
import sys
import urllib.parse
import urllib.request
import zipfile

ROOT = os.path.normpath(os.path.join(os.path.dirname(__file__), ".."))
# Overridable so a refetch can be staged + diffed against the pinned snapshots
# before replacing them (OURSPACE_SNAPSHOT_DIR=/tmp/stage fetch_snapshots.py …).
SNAP = os.environ.get("OURSPACE_SNAPSHOT_DIR") or os.path.join(ROOT, "data", "snapshots")
UA = {"User-Agent": "our-space-snapshot-fetch/1.0 (research; see repo README)"}


def _get(url: str, timeout: int = 600) -> bytes:
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.read()


def _save(rel: str, data: bytes) -> str:
    path = os.path.join(SNAP, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        f.write(data)
    print(f"  wrote {rel}  ({len(data):,} bytes)")
    return path


def _download(url: str, rel: str) -> None:
    print(f"  GET {url}")
    _save(rel, _get(url))


def _socrata(host: str, dataset: str, params: dict) -> str:
    """A Socrata SODA resource URL (the same form the bake scripts curl)."""
    q = urllib.parse.urlencode(params, quote_via=urllib.parse.quote)
    return f"https://{host}/resource/{dataset}?{q}"


def _delegate(script: str, *args: str) -> None:
    """Run an existing dedicated fetch script (single source of truth)."""
    cmd = [sys.executable, os.path.join(ROOT, "tools", script), *args]
    print(f"  → {' '.join(cmd[1:])}")
    subprocess.run(cmd, check=True, cwd=ROOT)


# --------------------------------------------------------------- fetchers ---


def fetch_amnesty() -> None:
    # Amnesty International "Decode Surveillance NYC" (CC BY-NC-ND 4.0).
    _download(
        "https://raw.githubusercontent.com/amnesty-crisis-evidence-lab/"
        "decode-surveillance-nyc/main/data/counts_per_intersections.csv",
        "amnesty/counts_per_intersections.csv",
    )


def fetch_dahir() -> None:
    # Dahir et al. 2025 camera detections, Stanford Digital Repository (CC BY 4.0).
    _download(
        "https://stacks.stanford.edu/file/druid:jr882ny4955/map_data.csv",
        "dahir/map_data.csv",
    )


def fetch_boroughs() -> None:
    _download(
        "https://data.cityofnewyork.us/api/geospatial/gthc-hcne?method=export&format=GeoJSON",
        "boroughs/borough_boundaries.geojson",
    )


def fetch_deflock() -> None:
    # DeFlock ALPRs synced into OSM; the raw Overpass JSON export the ALPR bake
    # parses (crates/data-pipeline/src/alpr.rs). Same Manhattan bbox as the walk
    # network — the pinned snapshot's extent confirms it. (For a five-borough
    # ALPR layer, widen to 40.48,-74.28,40.93,-73.68 and rebake.)
    query = (
        "[out:json][timeout:120];"
        'node["man_made"="surveillance"]["surveillance:type"="ALPR"]'
        "(40.698,-74.022,40.882,-73.906);"
        "out body;"
    )
    print("  POST https://overpass-api.de/api/interpreter (ALPR nodes, Manhattan bbox)")
    req = urllib.request.Request(
        "https://overpass-api.de/api/interpreter",
        data=urllib.parse.urlencode({"data": query}).encode(),
        headers=UA,
    )
    with urllib.request.urlopen(req, timeout=300) as r:
        _save("deflock/alpr.json", r.read())


def fetch_osm_walk() -> None:
    # Manhattan walk network via Overpass (the README's hand query, scripted).
    # `way + recurse-down + out body` — the graph bake (graph_osm.rs) parses way
    # node-ref lists plus separate node elements; the highway keep-set is applied
    # pipeline-side, so the dump just grabs every highway way in the bbox.
    query = (
        "[out:json][timeout:300];"
        'way["highway"](40.698,-74.022,40.882,-73.906);'
        "(._;>;);"
        "out body;"
    )
    print("  POST https://overpass-api.de/api/interpreter (Manhattan walk network)")
    req = urllib.request.Request(
        "https://overpass-api.de/api/interpreter",
        data=urllib.parse.urlencode({"data": query}).encode(),
        headers=UA,
    )
    with urllib.request.urlopen(req, timeout=600) as r:
        _save("osm/manhattan_walk.json", r.read())


def fetch_osm_cscl() -> None:
    # NYC Street Centerline (CSCL) — the citywide drive-graph source + bridges.
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "inkn-q76z.geojson",
            {
                "$select": "the_geom,rw_type,trafdir,nonped,posted_speed,full_street_name",
                "$limit": "200000",
            },
        ),
        "osm/cscl.geojson",
    )
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "inkn-q76z.geojson",
            {
                "$select": "the_geom,stname_label",
                "$where": "stname_label like '%BRG%'",
                "$limit": "50000",
            },
        ),
        "osm/cscl_bridges.geojson",
    )


def fetch_dot() -> None:
    # NYC DOT Traffic Management Center camera feed (locations only).
    _download("https://webcams.nyctmc.org/api/cameras/", "dot/cameras.json")


def fetch_enforcement() -> None:
    _delegate("fetch_enforcement.py")


def fetch_facilities() -> None:
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "ji82-xba5.json",
            {
                "$select": "facname,latitude,longitude,boro,facgroup,facsubgrp,factype,address",
                "$where": "facgroup in('SCHOOLS (K-12)','LIBRARIES')",
                "$limit": "5000",
            },
        ),
        "facilities/facilities.json",
    )


def fetch_linknyc() -> None:
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "n6c5-95xh.json",
            {
                "$select": "latitude,longitude,status,kiosk_type",
                "$where": "boro='Manhattan'",
                "$limit": "5000",
            },
        ),
        "linknyc/kiosks_manhattan.json",
    )


def fetch_gtfs() -> None:
    # Subway feed + station extract (dedicated script)…
    _delegate("fetch_gtfs.py")
    # …plus the Manhattan bus GTFS the ACE/bus-day bakes read (subway-only script
    # doesn't cover it; recipe previously lived only in bake_citywide.sh).
    url = "https://rrgtfsfeeds.s3.amazonaws.com/gtfs_m.zip"
    print(f"  GET {url}")
    raw = _get(url)
    _save("gtfs/gtfs_m.zip", raw)
    outdir = os.path.join(SNAP, "gtfs", "gtfs_m")
    shutil.rmtree(outdir, ignore_errors=True)
    with zipfile.ZipFile(io.BytesIO(raw)) as z:
        z.extractall(outdir)
    print(f"  unzipped → gtfs/gtfs_m/ ({len(os.listdir(outdir))} files)")


def fetch_ace_routes() -> None:
    # MTA Bus Automated Camera Enforced (ACE/ABLE) routes, data.ny.gov.
    _download(
        _socrata(
            "data.ny.gov",
            "ki2b-sg5y.json",
            {"$select": "route,program,implementation_date", "$limit": "1000"},
        ),
        "gtfs/ace_routes.json",
    )


def fetch_neighborhoods() -> None:
    # Pedia Cities NYC neighborhoods (see SOURCE.txt) — pinned March-2018 file.
    _download(
        "https://raw.githubusercontent.com/HodgesWardElliott/custom-nyc-neighborhoods/"
        "master/custom-pedia-cities-nyc-Mar2018.geojson",
        "neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson",
    )


def fetch_open_streets() -> None:
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "uiay-nctu.geojson",
            {
                "$select": "the_geom,appronstre,reviewstat,boroughname,apprdayswe",
                "$limit": "2000",
            },
        ),
        "open_streets/open_streets.geojson",
    )


def fetch_parks() -> None:
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "enfh-gkve.geojson",
            {
                "$select": "multipolygon,signname,typecategory,borough,acres",
                "$limit": "10000",
            },
        ),
        "parks/parks.geojson",
    )


def fetch_plazas() -> None:
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "k5k6-6jex.geojson",
            {"$select": "the_geom,plazaname,boroname", "$limit": "5000"},
        ),
        "plazas/plazas.geojson",
    )


def fetch_robotability() -> None:
    # IRL-CT Robotability Score, per-sidewalk GeoJSON.
    _download(
        "https://raw.githubusercontent.com/IRL-CT/robotability/main/public/data/sidewalks.geojson",
        "robotability/sidewalks.geojson",
    )


def fetch_teslas() -> None:
    _download(
        "https://raw.githubusercontent.com/fedhere/PUI2015_EC/master/mam1612_EC/"
        "nyc-zip-code-tabulation-areas-polygons.geojson",
        "teslas/nyc_zips.geojson",
    )
    _download(
        _socrata(
            "data.ny.gov",
            "w4pv-hbkt.csv",
            {
                "$select": "zip,count(*) as n",
                "$where": "make='TESLA' AND county in"
                "('NEW YORK','KINGS','QUEENS','BRONX','RICHMOND')",
                "$group": "zip",
                "$limit": "5000",
            },
        ),
        "teslas/tesla_by_zip.csv",
    )


def fetch_tiger() -> None:
    # Census TIGER/Line 2020 block groups, New York (state FIPS 36).
    _download(
        "https://www2.census.gov/geo/tiger/TIGER2020/BG/tl_2020_36_bg.zip",
        "tiger/tl_2020_36_bg.zip",
    )


def fetch_census() -> None:
    # BG centroids (no key) + ACS demographics (needs CENSUS_API_KEY) — the two
    # stats-pipeline files, via the dedicated script…
    _delegate("fetch_census.py", "centroids")
    _delegate("fetch_census.py", "acs")
    # …plus the equity-bake pair (blockgroups.geojson + acs.json), which no
    # script produced before: TIGERweb BG polygons for Manhattan (36061) and
    # ACS 5-year B03002 race/ethnicity keyed by 12-digit GEOID.
    print("  TIGERweb: block-group polygons, county 36061")
    feats: list = []
    offset = 0
    while True:
        url = (
            "https://tigerweb.geo.census.gov/arcgis/rest/services/TIGERweb/"
            "Tracts_Blocks/MapServer/1/query?"
            + urllib.parse.urlencode(
                {
                    "where": "STATE='36' AND COUNTY='061'",
                    "outFields": "GEOID",
                    "f": "geojson",
                    "resultOffset": str(offset),
                    "resultRecordCount": "1000",
                }
            )
        )
        page = json.loads(_get(url))
        got = page.get("features", [])
        feats.extend(got)
        if len(got) < 1000:
            break
        offset += 1000
    _save(
        "census/blockgroups.geojson",
        json.dumps({"type": "FeatureCollection", "features": feats}).encode(),
    )

    # Same key discovery as fetch_census.py: real env first, then ./.env.
    key = os.environ.get("CENSUS_API_KEY", "")
    if not key and os.path.exists(os.path.join(ROOT, ".env")):
        with open(os.path.join(ROOT, ".env")) as f:
            for line in f:
                if line.strip().startswith("CENSUS_API_KEY="):
                    key = line.split("=", 1)[1].strip().strip('"').strip("'")
    if not key:
        print("  ! CENSUS_API_KEY not set (env or .env) — acs.json fetch will likely 400")
    vars_ = "B03002_001E,B03002_003E,B03002_004E,B03002_006E,B03002_012E"
    url = (
        "https://api.census.gov/data/2022/acs/acs5?"
        + urllib.parse.urlencode(
            {
                "get": vars_,
                "for": "block group:*",
                "in": "state:36 county:061",
                **({"key": key} if key else {}),
            }
        )
    )
    print("  Census API: ACS5 B03002 for county 36061 block groups")
    rows = json.loads(_get(url))
    head = rows[0]
    idx = {name: head.index(name) for name in vars_.split(",")}
    out = {}
    for row in rows[1:]:
        geoid = row[-4] + row[-3] + row[-2] + row[-1]  # state+county+tract+bg
        num = lambda v: float(v) if v not in (None, "") else 0.0  # noqa: E731
        out[geoid] = {
            "total": num(row[idx["B03002_001E"]]),
            "white": num(row[idx["B03002_003E"]]),
            "black": num(row[idx["B03002_004E"]]),
            "asian": num(row[idx["B03002_006E"]]),
            "hispanic": num(row[idx["B03002_012E"]]),
        }
    _save("census/acs.json", json.dumps(out, indent=2).encode())


def fetch_crime() -> None:
    _delegate("fetch_crime.py")
    _delegate("fetch_311.py")


def fetch_lodes() -> None:
    _delegate("fetch_lodes.py")  # needs `duckdb` CLI on PATH


def fetch_violations() -> None:
    # NYC Open Parking & Camera Violations (nc67-uf89): the two aggregates the
    # ACE "surveillance evidence" callout was transcribed from
    # (app-interactive/src/main.rs ACE_BUS_LANE_* constants). No coordinates in
    # the dataset; county='NY' = Manhattan. Refetching may change the numbers —
    # re-transcribe the constants if so.
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "nc67-uf89.json",
            {
                "$select": "violation,count(*) as n,sum(fine_amount) as fines",
                "$where": "county='NY' AND violation in"
                "('NO STANDING-BUS LANE','FAILURE TO STOP AT RED LIGHT')",
                "$group": "violation",
            },
        ),
        "violations/bus_lane_manhattan.json",
    )
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "nc67-uf89.json",
            {
                "$select": "issuing_agency,count(*) as n",
                "$where": "county='NY' AND violation='NO STANDING-BUS LANE'",
                "$group": "issuing_agency",
                "$order": "n DESC",
            },
        ),
        "violations/bus_lane_issuing_agency.json",
    )


def fetch_tlc() -> None:
    # Taxi-zone polygons (shapefile zip + GeoJSON conversion via geopandas)…
    url = "https://d37ci6vzurychx.cloudfront.net/misc/taxi_zones.zip"
    print(f"  GET {url}")
    raw = _get(url)
    _save("tlc/taxi_zones.zip", raw)
    try:
        import geopandas as gpd  # project dep (pyproject.toml)

        gdf = gpd.read_file(os.path.join(SNAP, "tlc", "taxi_zones.zip")).to_crs(4326)
        gdf.to_file(os.path.join(SNAP, "tlc", "taxi_zones.geojson"), driver="GeoJSON")
        print("  wrote tlc/taxi_zones.geojson (EPSG:4326 via geopandas)")
    except ImportError:
        print("  ! geopandas unavailable — taxi_zones.geojson NOT regenerated")
    # …plus the trip aggregates (DuckDB over the TLC HV-FHV parquet; the remote
    # scan streams only the relevant row groups). Recipes are verbatim from
    # README.md §TLC and tools/bake_citywide.sh — same day, same columns.
    if shutil.which("duckdb") is None:
        print("  ! duckdb CLI not found — skipping trip CSVs (see README §TLC)")
        return
    pq = "https://d37ci6vzurychx.cloudfront.net/trip-data/fhvhv_tripdata_2024-06.parquet"
    day = "2024-06-25"
    # Manhattan LocationIDs, straight from the zone polygons we just fetched.
    with open(os.path.join(SNAP, "tlc", "taxi_zones.geojson"), "rb") as f:
        zones = json.load(f)
    ids = sorted(
        int(ft["properties"]["LocationID"])
        for ft in zones["features"]
        if ft["properties"].get("borough") == "Manhattan"
    )
    ids_sql = ",".join(map(str, ids))
    httpfs = "INSTALL httpfs; LOAD httpfs; "
    trips_all = os.path.join(SNAP, "tlc", "taxi_trips_all.csv")
    trips_nyc = os.path.join(SNAP, "tlc", "taxi_trips_all_nyc.csv")
    day_rows = (
        "SELECT CAST(date_part('hour',pickup_datetime)*60+date_part('minute',pickup_datetime) AS INTEGER) AS pu_min, "
        "PULocationID, DOLocationID, "
        "CAST(GREATEST(1,date_diff('minute',pickup_datetime,dropoff_datetime)) AS INTEGER) AS dur_min, "
        "ROUND(trip_miles,3) AS trip_miles, CAST(trip_time AS INTEGER) AS trip_time "
        f"FROM read_parquet('{pq}') "
        f"WHERE pickup_datetime >= TIMESTAMP '{day} 00:00:00' "
        f"AND pickup_datetime < TIMESTAMP '{day} 00:00:00' + INTERVAL 1 DAY "
        "AND PULocationID <> DOLocationID"
    )
    jobs = [
        # one real day at trip granularity, Manhattan↔Manhattan
        (
            "tlc/taxi_trips_all.csv",
            f"COPY ({day_rows} AND PULocationID IN ({ids_sql}) AND DOLocationID IN ({ids_sql}) "
            f"ORDER BY pu_min) TO '{trips_all}' (HEADER)",
        ),
        (
            "tlc/taxi_perminute_od.csv",
            "COPY (SELECT pu_min, PULocationID, DOLocationID, COUNT(*) AS trips "
            f"FROM read_csv_auto('{trips_all}') GROUP BY 1,2,3) "
            f"TO '{os.path.join(SNAP, 'tlc', 'taxi_perminute_od.csv')}' (HEADER)",
        ),
        # citywide variants (all zone IDs ≤ 263)
        (
            "tlc/taxi_trips_all_nyc.csv",
            f"COPY ({day_rows} AND PULocationID <= 263 AND DOLocationID <= 263 "
            f"ORDER BY pu_min) TO '{trips_nyc}' (HEADER)",
        ),
        (
            "tlc/taxi_perminute_od_nyc.csv",
            "COPY (SELECT pu_min, PULocationID, DOLocationID, COUNT(*) AS trips "
            f"FROM read_csv_auto('{trips_nyc}') GROUP BY 1,2,3) "
            f"TO '{os.path.join(SNAP, 'tlc', 'taxi_perminute_od_nyc.csv')}' (HEADER)",
        ),
        # month-scale O-D matrix for the animated vehicle routes (HAVING >= 200)
        (
            "tlc/zone_od.csv",
            "COPY (SELECT PULocationID, DOLocationID, COUNT(*) AS trips "
            f"FROM read_parquet('{pq}') "
            f"WHERE PULocationID IN ({ids_sql}) AND DOLocationID IN ({ids_sql}) "
            "AND PULocationID <> DOLocationID "
            "GROUP BY 1,2 HAVING COUNT(*) >= 200 ORDER BY trips DESC) "
            f"TO '{os.path.join(SNAP, 'tlc', 'zone_od.csv')}' (HEADER)",
        ),
        # per-zone pickup counts for the dashcam density field (`loc,trips`)
        (
            "tlc/zone_trips.csv",
            "COPY (SELECT PULocationID AS loc, COUNT(*) AS trips "
            f"FROM read_parquet('{pq}') "
            f"WHERE PULocationID IN ({ids_sql}) GROUP BY 1 ORDER BY 1) "
            f"TO '{os.path.join(SNAP, 'tlc', 'zone_trips.csv')}' (HEADER)",
        ),
    ]
    for rel, sql in jobs:
        print(f"  duckdb → {rel}")
        subprocess.run(["duckdb", "-c", httpfs + sql], check=True)


def fetch_buildings_heavy() -> None:
    # Manhattan footprints (Socrata, BIN 1xxxxxxx = Manhattan)…
    _download(
        _socrata(
            "data.cityofnewyork.us",
            "5zhs-2jue.geojson",
            {
                "$select": "the_geom,bin,height_roof,ground_elevation,feature_code",
                "$where": "bin between 1000000 and 1999999",
                "$limit": "50000",
            },
        ),
        "buildings/manhattan_footprints.geojson",
    )
    # …the ~916 MB LoD2 CityGML archive…
    _download("https://s-media.nyc.gov/agencies/oti/DA_WISE_GML.zip", "buildings/DA_WISE_GML.zip")
    # …and the derived landmark massings (regenerated from the two inputs).
    _delegate("extract_landmarks.py")


# --------------------------------------------------------------- registry ---

# name -> (files that must exist for "already fetched", fetcher, heavy?)
DATASETS: dict[str, tuple[list[str], object, bool]] = {
    "amnesty": (["amnesty/counts_per_intersections.csv"], fetch_amnesty, False),
    "boroughs": (["boroughs/borough_boundaries.geojson"], fetch_boroughs, False),
    "buildings": (
        [
            "buildings/manhattan_footprints.geojson",
            "buildings/DA_WISE_GML.zip",
            "buildings/landmarks_lod2.json",
        ],
        fetch_buildings_heavy,
        True,
    ),
    "census": (
        [
            "census/bg_centroids_nyc.csv",
            "census/acs_nyc.csv",
            "census/blockgroups.geojson",
            "census/acs.json",
        ],
        fetch_census,
        False,
    ),
    "crime": (
        ["crime/nypd_points.csv", "crime/nyc311_disorder_points.csv"],
        fetch_crime,
        False,
    ),
    "dahir": (["dahir/map_data.csv"], fetch_dahir, False),
    "deflock": (["deflock/alpr.json"], fetch_deflock, False),
    "dot": (["dot/cameras.json"], fetch_dot, False),
    "enforcement": (["enforcement/enforcement_signs.csv"], fetch_enforcement, False),
    "facilities": (["facilities/facilities.json"], fetch_facilities, False),
    "gtfs": (
        ["gtfs/gtfs_m.zip", "gtfs/subway/gtfs_subway.zip", "gtfs/subway/stations_subway.csv"],
        fetch_gtfs,
        False,
    ),
    "ace_routes": (["gtfs/ace_routes.json"], fetch_ace_routes, False),
    "linknyc": (["linknyc/kiosks_manhattan.json"], fetch_linknyc, False),
    "lodes": (
        [
            "lodes/ny_od_main_JT00_2022.csv.gz",
            "lodes/ny_od_aux_JT00_2022.csv.gz",
            "lodes/bg_od_nyc.csv",
        ],
        fetch_lodes,
        True,
    ),
    "neighborhoods": (
        ["neighborhoods/custom-pedia-cities-nyc-Mar2018.geojson"],
        fetch_neighborhoods,
        False,
    ),
    "open_streets": (["open_streets/open_streets.geojson"], fetch_open_streets, False),
    "osm_walk": (["osm/manhattan_walk.json"], fetch_osm_walk, False),
    "osm_cscl": (["osm/cscl.geojson", "osm/cscl_bridges.geojson"], fetch_osm_cscl, False),
    "parks": (["parks/parks.geojson"], fetch_parks, False),
    "plazas": (["plazas/plazas.geojson"], fetch_plazas, False),
    "robotability": (["robotability/sidewalks.geojson"], fetch_robotability, False),
    "teslas": (["teslas/nyc_zips.geojson", "teslas/tesla_by_zip.csv"], fetch_teslas, False),
    "tiger": (["tiger/tl_2020_36_bg.zip"], fetch_tiger, False),
    "tlc": (
        [
            "tlc/taxi_zones.zip",
            "tlc/taxi_zones.geojson",
            "tlc/taxi_trips_all.csv",
            "tlc/taxi_perminute_od.csv",
            "tlc/taxi_trips_all_nyc.csv",
            "tlc/taxi_perminute_od_nyc.csv",
            "tlc/zone_od.csv",
            "tlc/zone_trips.csv",
        ],
        fetch_tlc,
        True,
    ),
    "violations": (
        ["violations/bus_lane_manhattan.json", "violations/bus_lane_issuing_agency.json"],
        fetch_violations,
        False,
    ),
}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("datasets", nargs="*", help="dataset names, or 'all'")
    ap.add_argument("--list", action="store_true", help="list datasets + files and exit")
    ap.add_argument("--force", action="store_true", help="refetch even if files exist")
    ap.add_argument("--heavy", action="store_true", help="with 'all': include multi-GB datasets")
    args = ap.parse_args()

    if args.list or not args.datasets:
        for name, (files, _, heavy) in DATASETS.items():
            have = all(os.path.exists(os.path.join(SNAP, f)) for f in files)
            tag = "heavy " if heavy else ""
            print(f"{'✓' if have else '·'} {name:14} {tag}→ {', '.join(files)}")
        return 0

    wanted = list(DATASETS) if args.datasets == ["all"] else args.datasets
    unknown = [n for n in wanted if n not in DATASETS]
    if unknown:
        ap.error(f"unknown dataset(s): {', '.join(unknown)} (use --list)")

    failures = []
    for name in wanted:
        files, fn, heavy = DATASETS[name]
        if args.datasets == ["all"] and heavy and not args.heavy:
            print(f"— {name}: heavy, skipped (pass --heavy to include)")
            continue
        if not args.force and all(os.path.exists(os.path.join(SNAP, f)) for f in files):
            print(f"✓ {name}: all files present (use --force to refetch)")
            continue
        print(f"⇣ {name}:")
        try:
            fn()
        except Exception as e:  # keep going; report at the end
            failures.append((name, str(e)))
            print(f"  ✗ {name} FAILED: {e}")
    if failures:
        print("\nFailed:")
        for name, err in failures:
            print(f"  {name}: {err}")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
