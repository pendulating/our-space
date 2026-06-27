#!/usr/bin/env python3
"""Generate 3D massing for NYC's iconic bridges as landmark-format JSON.

The output reuses the *exact* landmark schema (`{landmarks:[{name,bin,height_m,
surfaces:[{type,ring:[[lon,lat,h]]}]}]}`) so `data-pipeline bake-landmarks` and the
app's `landmark_massing_mesh` render it with **zero new Rust geometry**. Each bridge
becomes a set of oblique-shaded surfaces:

  * deck   — a raised ribbon (top RoofSurface + two side WallSurfaces for thickness),
             traced from the bridge's true CSCL centerline, clipped to the main span.
  * towers — portal frames (two vertical legs straddling the deck + a crossbeam),
             from a curated height/position table. Cantilever bridges get solid piers.
  * cables — for suspension bridges, a parabolic main cable between tower tops plus
             back-spans to the anchorages, drawn as thin vertical ribbons.

Heights are in real metres above the base; the renderer scales them by
`LANDMARK_HEIGHT` (0.5), matching the curated building landmarks so bridges and
skyline read at one consistent vertical scale.

Input : a CSCL bridge GeoJSON (MultiLineString features keyed by `stname_label`).
Run   : python3 tools/generate_bridges.py <cscl_bridges.geojson> <out_bridges.json>
"""
import json
import math
import sys

# ── curated bridge table ──────────────────────────────────────────────────────
# name      : CSCL `stname_label` to gather segments from
# disp      : display name
# style     : "suspension" (main cables) | "cantilever" (piers, no cables)
# deck_w    : deck width  (m)   deck_h : clearance above water (m)
# tower_h   : tower height above water (m)    span : tower-to-tower distance (m)
# shift     : metres to slide the tower pair along the deck axis off the geometric
#             midpoint (to centre them on the true water crossing); + is toward the
#             axis's positive end.
BRIDGES = [
    # East River suspension trio
    dict(name="BROOKLYN BRG",      disp="Brooklyn Bridge",   style="suspension",
         deck_w=26, deck_h=41, tower_h=84,  span=486, shift=0),
    dict(name="MANHATTAN BRG",     disp="Manhattan Bridge",  style="suspension",
         deck_w=37, deck_h=41, tower_h=102, span=448, shift=0),
    dict(name="WILLIAMSBURG BRG",  disp="Williamsburg Bridge", style="suspension",
         deck_w=36, deck_h=41, tower_h=101, span=488, shift=0),
    # East River cantilever — humped double-cantilever truss, no cables
    dict(name="ED KOCH QUEENSBORO BRG", disp="Queensboro Bridge", style="cantilever",
         deck_w=30, deck_h=40, tower_h=107, span=360, shift=0,
         truss=dict(pier=58.0, mid=18.0, end=4.0)),
    # Hudson
    dict(name="GEORGE WASHINGTON BRG", disp="George Washington Bridge", style="suspension",
         deck_w=36, deck_h=65, tower_h=184, span=1067, shift=0),
    # The Narrows
    dict(name="VERRAZZANO BRG",    disp="Verrazzano-Narrows Bridge", style="suspension",
         deck_w=31, deck_h=69, tower_h=211, span=1298, shift=0),
    # East River / Long Island Sound suspension
    dict(name="THROGS NECK BRG",   disp="Throgs Neck Bridge", style="suspension",
         deck_w=23, deck_h=43, tower_h=110, span=549, shift=0),
    dict(name="WHITESTONE BRG",    disp="Bronx–Whitestone Bridge", style="suspension",
         deck_w=23, deck_h=46, tower_h=115, span=701, shift=0),
    # RFK is three crossings meeting at Randall's/Ward's Island; PCA over the whole
    # complex smears one straight span diagonally across the island. Anchor the iconic
    # East River (Hell Gate) suspension span explicitly so the towers land on the
    # Astoria↔Ward's Island water crossing — 1,380 ft (421 m) span, 335 ft (102 m)
    # towers, 135 ft (41 m) clearance (Ammann, 1936).
    dict(name="ROBERT F KENNEDY BRG", disp="RFK Bridge", style="suspension",
         deck_w=24, deck_h=41, tower_h=102, span=421, shift=0,
         anchor=((-73.9259, 40.7785), (-73.9285, 40.7817))),
]

DECK_TH = 3.0    # deck slab thickness (m)
LEG = 11.0       # tower leg footprint (m, square)
BEAM = 14.0      # crossbeam vertical extent at the tower top (m)
CABLE_R = 1.6    # main-cable ribbon half-height (m)
MLAT = 111320.0  # metres per degree latitude


def to_local(pts, lon0, lat0):
    """(lon,lat) → local metres about (lon0,lat0), equirectangular at lat0."""
    mlon = MLAT * math.cos(math.radians(lat0))
    return [((lon - lon0) * mlon, (lat - lat0) * MLAT) for lon, lat in pts]


def to_lonlat(x, y, lon0, lat0):
    mlon = MLAT * math.cos(math.radians(lat0))
    return [lon0 + x / mlon, lat0 + y / MLAT]


def norm(vx, vy):
    m = math.hypot(vx, vy) or 1.0
    return vx / m, vy / m


def pca_axis(pts):
    """Principal axis (unit deck direction) of a set of local (x,y) points."""
    n = len(pts)
    cx = sum(p[0] for p in pts) / n
    cy = sum(p[1] for p in pts) / n
    sxx = syy = sxy = 0.0
    for x, y in pts:
        dx, dy = x - cx, y - cy
        sxx += dx * dx
        syy += dy * dy
        sxy += dx * dy
    # principal eigenvector of [[sxx,sxy],[sxy,syy]]
    theta = 0.5 * math.atan2(2 * sxy, sxx - syy)
    return (cx, cy), (math.cos(theta), math.sin(theta))


def box_surfaces(cx, cy, u, v, hu, hv, h0, h1):
    """Axis-aligned (to u,v) box → 4 walls + 1 roof, rings in local [x,y,h]."""
    ux, uy = u
    vx, vy = v
    corner = lambda su, sv: (cx + su * hu * ux + sv * hv * vx,
                             cy + su * hu * uy + sv * hv * vy)
    c = [corner(-1, -1), corner(1, -1), corner(1, 1), corner(-1, 1)]
    surfs = [dict(type="RoofSurface", ring=[[x, y, h1] for x, y in c])]
    for i in range(4):
        a, b = c[i], c[(i + 1) % 4]
        surfs.append(dict(type="WallSurface", ring=[
            [a[0], a[1], h0], [b[0], b[1], h0], [b[0], b[1], h1], [a[0], a[1], h1]]))
    return surfs


def cable_ribbon(curve):
    """Polyline of (x,y,h) → a vertical ribbon tracing the cable. Tagged
    RoofSurface so the north-facing-wall cull never punches gaps in the catenary;
    its near-vertical normal still shades it a dark hairline on the white ground."""
    out = []
    for (x0, y0, h0), (x1, y1, h1) in zip(curve, curve[1:]):
        out.append(dict(type="RoofSurface", ring=[
            [x0, y0, h0 + CABLE_R], [x1, y1, h1 + CABLE_R],
            [x1, y1, h1 - CABLE_R], [x0, y0, h0 - CABLE_R]]))
    return out


def suspender(x, y, h_lo, h_hi, u):
    """A vertical hanger from the deck up to the main cable — a thin fin oriented
    along the deck axis (RoofSurface, so it's never culled)."""
    w = 1.3
    ux, uy = u
    return dict(type="RoofSurface", ring=[
        [x - ux * w, y - uy * w, h_lo], [x + ux * w, y + uy * w, h_lo],
        [x + ux * w, y + uy * w, h_hi], [x - ux * w, y - uy * w, h_hi]])


def deck_ribbon(a, b, deck_w, deck_h):
    """One straight deck slab a→b (local xy): top RoofSurface + two side WallSurfaces
    for slab thickness."""
    dx, dy = norm(b[0] - a[0], b[1] - a[1])
    nx, ny = -dy, dx
    hw = deck_w / 2
    aL = (a[0] + nx * hw, a[1] + ny * hw)
    bL = (b[0] + nx * hw, b[1] + ny * hw)
    aR = (a[0] - nx * hw, a[1] - ny * hw)
    bR = (b[0] - nx * hw, b[1] - ny * hw)
    lo = deck_h - DECK_TH
    return [
        dict(type="RoofSurface", ring=[
            [aL[0], aL[1], deck_h], [bL[0], bL[1], deck_h],
            [bR[0], bR[1], deck_h], [aR[0], aR[1], deck_h]]),
        dict(type="WallSurface", ring=[
            [aL[0], aL[1], deck_h], [bL[0], bL[1], deck_h],
            [bL[0], bL[1], lo], [aL[0], aL[1], lo]]),
        dict(type="WallSurface", ring=[
            [aR[0], aR[1], deck_h], [bR[0], bR[1], deck_h],
            [bR[0], bR[1], lo], [aR[0], aR[1], lo]]),
    ]


def truss_web(curve, base):
    """Polyline of (x,y,h_top) → a solid vertical web from `base` up to the top
    chord. Tagged RoofSurface so the north-facing-wall cull leaves the cantilever
    silhouette intact (its near-vertical normal still shades a dark hairline)."""
    out = []
    for (x0, y0, _h0), (x1, y1, h1) in zip(curve, curve[1:]):
        out.append(dict(type="RoofSurface", ring=[
            [x0, y0, base], [x1, y1, base], [x1, y1, h1], [x0, y0, _h0]]))
    return out


def cantilever_profile(t, t1, t2, t_lo, t_hi, mid, hp, hm, he):
    """Top-chord height *above the deck* of a double-cantilever truss at along-axis
    station `t`: rises to `hp` over each main pier (t1,t2), dips to `hm` at the
    suspended mid-span, tapers to `he` at the approach ends (t_lo,t_hi) — the classic
    twin-hump Queensboro silhouette over the full shore-to-shore deck."""
    if t < t1:                       # west approach arm: pier → west deck end
        f = min(1.0, max(0.0, (t1 - t) / ((t1 - t_lo) or 1.0)))
        return hp + (he - hp) * f
    if t > t2:                       # east approach arm: pier → east deck end
        f = min(1.0, max(0.0, (t - t2) / ((t_hi - t2) or 1.0)))
        return hp + (he - hp) * f
    c = (t - mid) / (((t2 - t1) / 2) or 1.0)   # -1..1 between the piers
    return hm + (hp - hm) * (c * c)            # hm at center → hp at the piers


def build_bridge(spec, segs, lon0, lat0):
    """segs: list of polylines (each a list of (lon,lat)). Returns surface list
    (rings in LOCAL metres) — caller converts to lon/lat."""
    deck_w, deck_h, tower_h = spec["deck_w"], spec["deck_h"], spec["tower_h"]
    style = spec["style"]

    local = [to_local(s, lon0, lat0) for s in segs]
    allpts = [p for s in local for p in s]
    surfaces = []

    anchor = spec.get("anchor")
    if anchor:
        # explicit main-span anchor (two lon/lat tower points): used where the CSCL
        # geometry is a branched complex (RFK = three crossings at Randall's/Ward's
        # Island) and PCA-over-everything would smear one straight span diagonally
        # across the wrong place. Axis + span come straight from the anchor.
        la = to_local([anchor[0]], lon0, lat0)[0]
        lb = to_local([anchor[1]], lon0, lat0)[0]
        cx, cy = (la[0] + lb[0]) / 2, (la[1] + lb[1]) / 2
        u = norm(lb[0] - la[0], lb[1] - la[1])
        span, mid = math.hypot(lb[0] - la[0], lb[1] - la[1]), 0.0
        tc1, tc2 = la, lb
    else:
        (cx, cy), u = pca_axis(allpts)
        span, mid = spec["span"], spec["shift"]
        center = lambda t: (cx + t * u[0], cy + t * u[1])
        tc1, tc2 = center(-span / 2 + mid), center(span / 2 + mid)
    v = (-u[1], u[0])
    proj = lambda p: (p[0] - cx) * u[0] + (p[1] - cy) * u[1]  # signed dist along axis
    perp = lambda p: (p[0] - cx) * v[0] + (p[1] - cy) * v[1]  # signed dist across axis
    CORRIDOR = 120.0  # the named-CSCL bridge centerline is ramp-free (lanes spread
                      # <35 m); a wide perpendicular corridor renders the FULL deck
                      # shore-to-shore (approaches included) while still dropping the
                      # odd stray off-axis spur — no more mid-river stub.

    # ── deck: trace the full true centerline so the span runs all the way to its
    # abutments / street grid (approaches included). A non-anchored span drops the odd
    # off-axis spur via the perpendicular corridor; an anchored *branched* complex
    # (RFK = three crossings at Randall's/Ward's Island) traces every arm so each one
    # connects to its borough's streets. The anchored structure (towers/cables) still
    # rides the main span via tc1/tc2 above.
    t_lo, t_hi = -span / 2, span / 2   # along-axis extent of the actual deck
    decked = []
    for poly in local:
        for a, b in zip(poly, poly[1:]):
            if not anchor and abs(perp(a)) > CORRIDOR and abs(perp(b)) > CORRIDOR:
                continue
            surfaces += deck_ribbon(a, b, deck_w, deck_h)
            decked += [proj(a), proj(b)]
    if decked:
        t_lo, t_hi = min(decked), max(decked)

    edge = lambda c, s: (c[0] + s * (deck_w / 2) * v[0], c[1] + s * (deck_w / 2) * v[1])

    if style == "cantilever":
        # along-axis stations of the two main piers
        t1, t2 = proj(tc1), proj(tc2)
        if t1 > t2:
            t1, t2 = t2, t1
        # support piers (water → deck) under each main pier
        for tc in (tc1, tc2):
            surfaces += box_surfaces(tc[0], tc[1], u, v, LEG * 0.6, deck_w / 2,
                                     0.0, deck_h)
        # portal towers (legs + crossbeam) rising above the deck at each pier
        for tc in (tc1, tc2):
            for s in (-1, 1):
                lc = edge(tc, s)
                surfaces += box_surfaces(lc[0], lc[1], u, v, LEG / 2, LEG / 2, 0.0, tower_h)
            surfaces += box_surfaces(tc[0], tc[1], u, v, LEG / 2, deck_w / 2 + LEG / 2,
                                     tower_h - BEAM, tower_h)
        # cantilever truss web along each deck edge — humped over the piers, dipping
        # to the suspended mid-span, tapering to the approaches (no cables).
        tr = spec.get("truss", dict(pier=45.0, mid=22.0, end=6.0))
        for s in (-1, 1):
            web = []
            N = 64
            for k in range(N + 1):
                t = t_lo + (t_hi - t_lo) * k / N
                top = deck_h + cantilever_profile(t, t1, t2, t_lo, t_hi, mid,
                                                  tr["pier"], tr["mid"], tr["end"])
                x = cx + t * u[0] + s * (deck_w / 2) * v[0]
                y = cy + t * u[1] + s * (deck_w / 2) * v[1]
                web.append((x, y, top))
            surfaces += truss_web(web, deck_h)
    else:
        # portal towers: two legs straddling the deck + a crossbeam over the roadway
        for tc in (tc1, tc2):
            for s in (-1, 1):
                lc = edge(tc, s)
                surfaces += box_surfaces(lc[0], lc[1], u, v, LEG / 2, LEG / 2, 0.0, tower_h)
            surfaces += box_surfaces(tc[0], tc[1], u, v, LEG / 2, deck_w / 2 + LEG / 2,
                                     tower_h - BEAM, tower_h)

        # ── main cables + back-spans, one per deck edge ──
        for s in (-1, 1):
            e1, e2 = edge(tc1, s), edge(tc2, s)
            sag = tower_h - (deck_h + 6.0)        # lowest point ~6 m over the deck
            N = 26
            main = []
            for k in range(N + 1):
                t = k / N
                x = e1[0] + (e2[0] - e1[0]) * t
                y = e1[1] + (e2[1] - e1[1]) * t
                h = tower_h - sag * (1 - (2 * t - 1) ** 2)   # parabola, towers→sag→towers
                main.append((x, y, h))
            surfaces += cable_ribbon(main)
            # suspender hangers from the main cable down to the deck (every other
            # node), so the span reads unmistakably as a suspension bridge.
            for x, y, h in main[2:-2:2]:
                if h - CABLE_R > deck_h + 1:
                    surfaces.append(suspender(x, y, deck_h, h - CABLE_R, u))
            # back-spans down to anchorages beyond each tower, at deck level
            bs = span * 0.42
            for tc, sign in ((tc1, -1), (tc2, 1)):
                ec = edge(tc, s)
                ax = ec[0] + sign * bs * u[0]
                ay = ec[1] + sign * bs * u[1]
                back = []
                M = 6
                for k in range(M + 1):
                    t = k / M
                    x = ec[0] + (ax - ec[0]) * t
                    y = ec[1] + (ay - ec[1]) * t
                    h = tower_h + (deck_h - tower_h) * t
                    back.append((x, y, h))
                surfaces += cable_ribbon(back)

    return surfaces


def main():
    if len(sys.argv) != 3:
        sys.exit("usage: generate_bridges.py <cscl_bridges.geojson> <out.json>")
    fc = json.load(open(sys.argv[1]))

    # gather segments per CSCL name
    by_name = {}
    for f in fc["features"]:
        nm = f["properties"].get("stname_label")
        g = f["geometry"]
        if not nm or g["type"] != "MultiLineString":
            continue
        for line in g["coordinates"]:
            by_name.setdefault(nm, []).append([(c[0], c[1]) for c in line])

    landmarks = []
    for spec in BRIDGES:
        segs = by_name.get(spec["name"])
        if not segs:
            print("  ! no CSCL segments for", spec["name"])
            continue
        flat = [p for s in segs for p in s]
        lon0 = sum(p[0] for p in flat) / len(flat)
        lat0 = sum(p[1] for p in flat) / len(flat)
        surfs = build_bridge(spec, segs, lon0, lat0)
        # local metres → lon/lat (keep per-vertex height)
        for su in surfs:
            su["ring"] = [to_lonlat(x, y, lon0, lat0) + [h] for x, y, h in su["ring"]]
        landmarks.append(dict(name=spec["disp"], bin="bridge",
                              height_m=float(spec["tower_h"]), surfaces=surfs))
        print(f"  {spec['disp']:32s} {len(segs):4d} segs -> {len(surfs):4d} surfaces")

    json.dump(dict(landmarks=landmarks), open(sys.argv[2], "w"))
    print(f"wrote {len(landmarks)} bridges -> {sys.argv[2]}")


if __name__ == "__main__":
    main()
