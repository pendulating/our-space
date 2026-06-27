#!/usr/bin/env python3
"""Fetch NYC automated photo-enforcement camera locations from the Street Sign Work
Orders dataset (qt6m-xctn): Current speed / bus-lane / red-light 'PHOTO ENFORCED' /
'CAMERA' signs in Manhattan. Converts state-plane coords (EPSG:2263, ftUS) to WGS84
via an inline Lambert-Conformal-Conic inverse (no pyproj dependency), dedups
co-located signs to distinct locations, and writes a lon,lat,subtype CSV for
`data-pipeline bake-enforcement`. Stdlib only.

Out: data/snapshots/enforcement/enforcement_signs.csv
"""
import math, json, urllib.parse, urllib.request, collections

# --- EPSG:2263 (NAD83 / NY Long Island, ftUS) -> WGS84 lat/lon, Lambert CC inverse ---
a = 6378137.0; f = 1/298.257222101; e2 = f*(2-f); e = math.sqrt(e2); FT = 0.3048006096
phi1 = math.radians(40+40/60); phi2 = math.radians(41+2/60)
phi0 = math.radians(40+10/60); lam0 = math.radians(-74.0); E0 = 300000.0  # false easting is 300000 METERS in EPSG:2263
t_of = lambda p: math.tan(math.pi/4 - p/2)/(((1-e*math.sin(p))/(1+e*math.sin(p)))**(e/2))
m_of = lambda p: math.cos(p)/math.sqrt(1-e2*math.sin(p)**2)
m1,m2 = m_of(phi1),m_of(phi2); t0,t1,t2 = t_of(phi0),t_of(phi1),t_of(phi2)
n = (math.log(m1)-math.log(m2))/(math.log(t1)-math.log(t2)); F = m1/(n*t1**n); rho0 = a*F*t0**n
def to_lonlat(xf, yf):
    E = xf*FT - E0; N = yf*FT
    rho = math.copysign(math.sqrt(E*E + (rho0-N)**2), n)
    t = (rho/(a*F))**(1/n); theta = math.atan2(E, rho0-N)
    lam = theta/n + lam0; phi = math.pi/2 - 2*math.atan(t)
    for _ in range(8):
        s = e*math.sin(phi); phi = math.pi/2 - 2*math.atan(t*(((1-s)/(1+s))**(e/2)))
    return math.degrees(lam), math.degrees(phi)

print("sanity Broadway@9th (989791,224853) ->", to_lonlat(989791,224853))

where = ("borough='Manhattan' AND record_type='Current' AND "
         "(upper(sign_description) like '%PHOTO ENFORCED%' OR upper(sign_description) like '%CAMERA%')")
url = "https://data.cityofnewyork.us/resource/qt6m-xctn.json?" + urllib.parse.urlencode(
    {"$select":"sign_description,sign_x_coord,sign_y_coord", "$where":where, "$limit":"50000"})
rows = json.load(urllib.request.urlopen(url, timeout=120))
print("fetched", len(rows), "rows")

def subtype(d):
    d = d.upper()
    if "RED LIGHT" in d: return "redlight"
    if "BUS" in d: return "bus"
    return "speed"

seen = {}
for r in rows:
    try: xf, yf = float(r["sign_x_coord"]), float(r["sign_y_coord"])
    except (KeyError, ValueError, TypeError): continue
    if xf <= 0 or yf <= 0: continue
    key = (round(xf), round(yf))  # exact-location dedup (collapses duplicate sign rows)
    if key in seen: continue
    lon, lat = to_lonlat(xf, yf)
    seen[key] = (lon, lat, subtype(r.get("sign_description","")))

out = "/Users/mattfranchi/Repos/our-space/data/snapshots/enforcement/enforcement_signs.csv"
with open(out, "w") as fh:
    fh.write("lon,lat,subtype\n")
    for lon,lat,st in seen.values():
        fh.write(f"{lon:.6f},{lat:.6f},{st}\n")
from collections import Counter
c = Counter(st for _,_,st in seen.values())
print("distinct locations:", len(seen), dict(c), "->", out)
