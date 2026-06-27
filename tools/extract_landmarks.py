#!/usr/bin/env python3
"""Extract LoD2 massing for a curated set of Manhattan landmarks from the NYC 3D
Building Model (CityGML, DA_WISE_GML.zip).

Pipeline: resolve each landmark's BIN by point-in-footprint against the building
footprints GeoJSON we already fetched → stream only the CityGML tiles that cover
them (via `unzip -p`, no multi-GB extraction) → pull each matching building's
wall/roof/ground surfaces → transform EPSG:2263 ftUS → WGS84 lon/lat + height (m
above the building's own base) → emit a compact JSON the Rust `bake-landmarks`
step turns into an ENU massing asset. Stdlib only (reuses the enforcement script's
inline Lambert-Conformal-Conic inverse).

Out: data/snapshots/buildings/landmarks_lod2.json
"""
import json, math, sys, subprocess
import xml.etree.ElementTree as ET

ROOT = "/Users/mattfranchi/Repos/our-space"
ZIP = f"{ROOT}/data/snapshots/buildings/DA_WISE_GML.zip"
FOOTPRINTS = f"{ROOT}/data/snapshots/buildings/manhattan_footprints.geojson"
OUT = f"{ROOT}/data/snapshots/buildings/landmarks_lod2.json"

# --- EPSG:2263 (NAD83 / NY Long Island, ftUS) -> WGS84, Lambert CC inverse ---
a = 6378137.0; f = 1/298.257222101; e2 = f*(2-f); e = math.sqrt(e2); FT = 0.3048006096
phi1 = math.radians(40+40/60); phi2 = math.radians(41+2/60)
phi0 = math.radians(40+10/60); lam0 = math.radians(-74.0); E0 = 300000.0
t_of = lambda p: math.tan(math.pi/4 - p/2)/(((1-e*math.sin(p))/(1+e*math.sin(p)))**(e/2))
m_of = lambda p: math.cos(p)/math.sqrt(1-e2*math.sin(p)**2)
m1, m2 = m_of(phi1), m_of(phi2); t0, t1, t2 = t_of(phi0), t_of(phi1), t_of(phi2)
n = (math.log(m1)-math.log(m2))/(math.log(t1)-math.log(t2)); F = m1/(n*t1**n); rho0 = a*F*t0**n
def to_lonlat(xf, yf):
    E = xf*FT - E0; N = yf*FT
    rho = math.copysign(math.sqrt(E*E + (rho0-N)**2), n)
    t = (rho/(a*F))**(1/n); theta = math.atan2(E, rho0-N)
    lam = theta/n + lam0; phi = math.pi/2 - 2*math.atan(t)
    for _ in range(8):
        s = e*math.sin(phi); phi = math.pi/2 - 2*math.atan(t*(((1-s)/(1+s))**(e/2)))
    return math.degrees(lam), math.degrees(phi)

def to_ftus(lon, lat):
    """WGS84 lon/lat -> EPSG:2263 ftUS (forward LCC; inverse of to_lonlat)."""
    lam = math.radians(lon); phi = math.radians(lat)
    t = t_of(phi); rho = a*F*t**n; theta = n*(lam - lam0)
    E = rho*math.sin(theta); N = rho0 - rho*math.cos(theta)
    return (E + E0)/FT, N/FT

# Curated landmarks: name -> (lon, lat) of a point ON the specific building, and the
# CityGML tile that covers it. (Points chosen to land on the target building's roof.)
LANDMARKS = [
    ("One World Trade Center", -74.01316, 40.71300, "DA12"),
    ("Flatiron Building",      -73.9897, 40.7411, "DA12"),
    ("Empire State Building",  -73.9857, 40.7484, "DA12"),
    ("Grand Central Terminal", -73.9772, 40.7527, "DA12"),
    ("30 Rockefeller Plaza",   -73.97957, 40.75911, "DA12"),
    ("Met Museum",             -73.9632, 40.7794, "DA13"),
    ("Apollo Theater",         -73.94992, 40.81012, "DA13"),
    ("The Cloisters",          -73.9319, 40.8649, "DA13"),
]

def point_in_ring(x, y, ring):
    inside = False; n = len(ring); j = n-1
    for i in range(n):
        xi, yi = ring[i][0], ring[i][1]; xj, yj = ring[j][0], ring[j][1]
        if ((yi > y) != (yj > y)) and (x < (xj-xi)*(y-yi)/(yj-yi) + xi):
            inside = not inside
        j = i
    return inside

def local(tag): return tag.rsplit('}', 1)[-1]

def building_bin(elem):
    for sa in elem.iter():
        if local(sa.tag) == "stringAttribute" and sa.get("name") == "BIN":
            for ch in sa:
                if local(ch.tag) == "value":
                    return (ch.text or "").strip()
    return None

def extract_surfaces(building):
    """Yield (surface_type, [(x_ft,y_ft,z_ft), ...]) for each LoD2 polygon ring."""
    for bb in building:
        if local(bb.tag) != "boundedBy":
            continue
        for surf in bb:
            stype = local(surf.tag)  # WallSurface / RoofSurface / GroundSurface
            if stype not in ("WallSurface", "RoofSurface", "GroundSurface"):
                continue
            for pl in surf.iter():
                if local(pl.tag) != "posList":
                    continue
                nums = [float(v) for v in (pl.text or "").split()]
                ring = [(nums[i], nums[i+1], nums[i+2]) for i in range(0, len(nums)-2, 3)]
                if len(ring) >= 3:
                    yield stype, ring

def ground_xy(building):
    """Yield GroundSurface rings as [(x_ft,y_ft), ...] only (cheap point-test pass)."""
    for bb in building:
        if local(bb.tag) != "boundedBy":
            continue
        for surf in bb:
            if local(surf.tag) != "GroundSurface":
                continue
            for pl in surf.iter():
                if local(pl.tag) != "posList":
                    continue
                nums = (pl.text or "").split()
                ring = [(float(nums[i]), float(nums[i+1])) for i in range(0, len(nums)-2, 3)]
                if len(ring) >= 3:
                    yield ring

def stream_tile(tile, pois):
    """Stream one tile; match each POI to the building whose GROUND footprint
    contains its (ftUS) point. `pois` = [(name, x_ft, y_ft)]. Returns {name: surfaces}."""
    member = f"DA_WISE_GMLs/{tile}_3D_Buildings_Merged.gml"
    proc = subprocess.Popen(["unzip", "-p", ZIP, member], stdout=subprocess.PIPE)
    pending = {nm: (x, y) for nm, x, y in pois}
    found = {}
    seen = 0
    ctx = ET.iterparse(proc.stdout, events=("end",))
    for _, elem in ctx:
        if local(elem.tag) != "Building":
            continue
        seen += 1
        if pending:
            hit = None
            for ring in ground_xy(elem):           # cheap: ground rings only
                xs = [p[0] for p in ring]; ys = [p[1] for p in ring]
                bx0, bx1, by0, by1 = min(xs), max(xs), min(ys), max(ys)
                for nm, (px, py) in pending.items():
                    if bx0 <= px <= bx1 and by0 <= py <= by1 and point_in_ring(px, py, ring):
                        hit = nm; break
                if hit: break
            if hit:
                surfaces = list(extract_surfaces(elem))  # full extract only on a hit
                found[hit] = (building_bin(elem), surfaces)
                print(f"    [{tile}] matched {hit}  BIN={found[hit][0]}  "
                      f"({len(surfaces)} surfaces)  [{len(found)}/{len(pois)}]")
                del pending[hit]
        elem.clear()
        if not pending:
            break
    proc.stdout.close(); proc.terminate()
    print(f"    [{tile}] scanned {seen} buildings; unmatched: {list(pending)}")
    return found

def main():
    by_tile = {}
    for name, lon, lat, tile in LANDMARKS:
        x, y = to_ftus(lon, lat)
        by_tile.setdefault(tile, []).append((name, x, y))
    landmarks = []
    for tile, pois in by_tile.items():
        print(f"== streaming {tile} for {len(pois)} landmark(s) ==")
        got = stream_tile(tile, pois)
        for name, (binv, surfaces) in got.items():
            allz = [p[2] for _, ring in surfaces for p in ring]
            base = min(allz) if allz else 0.0
            out_surfs = []
            for stype, ring in surfaces:
                pts = []
                for (x, y, z) in ring:
                    lon, lat = to_lonlat(x, y)
                    h_m = (z - base) * FT  # height above the building's own base
                    pts.append([round(lon, 7), round(lat, 7), round(h_m, 2)])
                out_surfs.append({"type": stype, "ring": pts})
            top = max((p[2] for s in out_surfs for p in s["ring"]), default=0)
            landmarks.append({"name": name, "bin": binv, "height_m": round(top, 1),
                              "surfaces": out_surfs})
            print(f"  {name:24} {len(out_surfs)} surfaces, peak {top:.0f} m")
    landmarks.sort(key=lambda l: l["name"])
    json.dump({"landmarks": landmarks}, open(OUT, "w"))
    print(f"== wrote {len(landmarks)} landmarks -> {OUT} ==")

if __name__ == "__main__":
    main()
