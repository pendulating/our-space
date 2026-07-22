#!/usr/bin/env python3
"""Fetch the MTA subway GTFS static feed (includes the Staten Island Railway) and the
NYC DOT Staten Island Ferry GTFS.

Three consumers:
  1. Station locations (stations_subway.csv) -- access/egress walk targets for the
     transit mode and the station covariate. Parent stops only (location_type=1).
  2. The full subway timetable tables -- input to `data-pipeline bake-subway`, which
     builds the headway-based subway-graph router (all-pairs station times / boardings /
     line-haul). The underground line-haul is comprehensively surveilled (MTA cameras
     in every station and car), and since 2026-07 the itinerary is *routed*, not
     approximated by crow-flies x circuity, so the bake needs stops/trips/stop_times/
     transfers/calendar, extracted alongside the raw zip.
  3. The Staten Island Ferry timetable -- bake-subway grafts it onto the subway graph
     as a real line (St George <-> Whitehall; SIR is rail-isolated, the ferry is the
     link), replacing the earlier hand-parameterized 15-min/25-min pseudo-route with
     the published schedule. nyc.gov sits behind Akamai and 403s non-browser agents,
     so the request carries a browser UA; the Mobility Database mirror (feed #518) is
     the fallback.

Out:
  data/snapshots/gtfs/subway/gtfs_subway.zip        (raw)
  data/snapshots/gtfs/subway/*.txt                  (extracted tables for bake-subway)
  data/snapshots/gtfs/subway/stations_subway.csv    (id,lat,lon)
  data/snapshots/gtfs/siferry/siferry-gtfs.zip      (raw)
  data/snapshots/gtfs/siferry/*.txt                 (extracted, flattened -- the zip
                                                     nests a versioned subdirectory)
"""
import csv
import io
import os
import sys
import urllib.request
import zipfile

URL = "https://rrgtfsfeeds.s3.amazonaws.com/gtfs_subway.zip"
DEST = "data/snapshots/gtfs/subway"
OUT = os.path.join(DEST, "stations_subway.csv")
# Tables bake-subway parses (shapes.txt is extracted too for future path viz).
TABLES = [
    "agency.txt",
    "calendar.txt",
    "calendar_dates.txt",
    "routes.txt",
    "stops.txt",
    "trips.txt",
    "stop_times.txt",
    "transfers.txt",
    "shapes.txt",
    "feed_info.txt",
]

FERRY_URLS = [
    "https://www.nyc.gov/html/dot/downloads/misc/siferry-gtfs.zip",
    # Mobility Database mirror of the same feed (mdb-518), in case nyc.gov moves it.
    "https://storage.googleapis.com/storage/v1/b/mdb-latest/o/"
    "us-new-york-staten-island-ferry-gtfs-518.zip?alt=media",
]
FERRY_DEST = "data/snapshots/gtfs/siferry"
UA = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36"


def fetch_ferry() -> None:
    os.makedirs(FERRY_DEST, exist_ok=True)
    zpath = os.path.join(FERRY_DEST, "siferry-gtfs.zip")
    if not os.path.exists(zpath):
        for url in FERRY_URLS:
            try:
                print(f"fetching {url} ...", file=sys.stderr)
                req = urllib.request.Request(url, headers={"User-Agent": UA})
                with urllib.request.urlopen(req, timeout=60) as r, open(zpath, "wb") as f:
                    f.write(r.read())
                break
            except Exception as e:  # noqa: BLE001 -- try the mirror
                print(f"  {e}; trying next source", file=sys.stderr)
        else:
            print("ferry GTFS unavailable; bake-subway will warn + fall back", file=sys.stderr)
            return
    n = 0
    with zipfile.ZipFile(zpath) as z:
        for name in z.namelist():
            base = os.path.basename(name)  # flatten the versioned inner directory
            if base.endswith(".txt"):
                with z.open(name) as src, open(os.path.join(FERRY_DEST, base), "wb") as dst:
                    dst.write(src.read())
                n += 1
    print(f"extracted {n} ferry tables -> {FERRY_DEST}", file=sys.stderr)


def main() -> int:
    os.makedirs(DEST, exist_ok=True)
    zpath = os.path.join(DEST, "gtfs_subway.zip")
    if not os.path.exists(zpath):
        print(f"fetching {URL} ...", file=sys.stderr)
        urllib.request.urlretrieve(URL, zpath)

    with zipfile.ZipFile(zpath) as z:
        names = set(z.namelist())
        for t in TABLES:
            if t not in names:
                print(f"  note: feed has no {t}; skipped", file=sys.stderr)
                continue
            with z.open(t) as src, open(os.path.join(DEST, t), "wb") as dst:
                dst.write(src.read())
        stops = z.read("stops.txt").decode("utf-8-sig")

    rows = []
    for r in csv.DictReader(io.StringIO(stops)):
        # location_type 1 = station complex (the parent of the N/S platform stops).
        if r.get("location_type") == "1":
            rows.append((r["stop_id"], float(r["stop_lat"]), float(r["stop_lon"])))
    rows.sort()
    # id,lat,lon only -- station names carry commas and the Rust reader splits on ','.
    with open(OUT, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["id", "lat", "lon"])
        for sid, lat, lon in rows:
            w.writerow([sid, f"{lat:.6f}", f"{lon:.6f}"])
    print(
        f"wrote {len(rows)} subway stations -> {OUT}; extracted {len(TABLES)} tables",
        file=sys.stderr,
    )
    fetch_ferry()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
