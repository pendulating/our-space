#!/usr/bin/env python3
"""Assemble data/derived/results/occlusion.json from the occlusion-audit artifacts.

Until 2026-07-14 occlusion.json was hand-assembled from terminal output, and its numbers were
computed on the DRIVE graph (the audit predated the walk-graph fix). This script makes it a
generated artifact on the same footing as everything else make_tables.py reads, computed on the
walk graph the paper actually uses.

Bake-level inputs (regenerate with; each takes ~a minute):
    B=target/release/batch; P=crates/app-interactive/assets/processed
    CENT=data/snapshots/census/bg_centroids_nyc.csv; E=data/derived/exposure
    for k in 1 2 4 8 16; do
      OURSPACE_RANGE_SCALE=$k "$B" occlusion-audit "$P/graph_nyc_walk.osgraph" "$CENT" \
          "$E/occl_audit_walk_x$k.csv" 10
    done
    OURSPACE_OCCLUSION=0 "$B" bg-exposure "$P/graph_nyc_walk.osgraph" "$CENT" \
        "$E/R_i_bg_nyc_walk_free.csv" 10           # free-space arm
    "$B" occlusion-probe "$P/graph_nyc_walk.osgraph" > "$E/occl_probe_walk.txt" 2>&1

Point-level attenuation = share of (camera, street-point) sightlines a facade blocks -- what a
PLACE-based measure would feel. Path-level = share of distinct cameras removed from R_i -- what a
PERSON feels. The gap is the paper's point-vs-path argument.

Writes data/derived/results/occlusion.json (+ copies the x1 per-BG audit to
data/derived/results/occlusion_audit_bg.csv, the file the results README documents).
"""
import csv, json, math, os, re, shutil, sys

import numpy as np

E = "data/derived/exposure"
OUT = "data/derived/results/occlusion.json"
SCALES = [1, 2, 4, 8, 16]
CCTV_RANGE_M, DOT_RANGE_M = 15, 30
NEED_BASE = ["jobs", "transit", "dens", "stations", "crime", "req311"]  # crime_ladder sample rule


def sum_audit(path):
    tot = dict.fromkeys(
        ["pairs_free", "pairs_occl", "groups_free", "groups_occl", "groups_saved", "groups_killed"], 0
    )
    with open(path) as f:
        for r in csv.DictReader(f):
            for k in tot:
                tot[k] += int(r[k])
    if tot["groups_free"] - tot["groups_occl"] != tot["groups_killed"]:
        sys.exit(f"{path}: killed != groups_free - groups_occl; audit output is inconsistent")
    return tot


range_sensitivity = []
for k in SCALES:
    t = sum_audit(f"{E}/occl_audit_walk_x{k}.csv")
    range_sensitivity.append({
        "range_scale": k,
        "cctv_range_m": CCTV_RANGE_M * k,
        "dot_range_m": DOT_RANGE_M * k,
        "point_level_attenuation": 1.0 - t["pairs_occl"] / t["pairs_free"],
        "path_level_attenuation": 1.0 - t["groups_occl"] / t["groups_free"],
        "cameras_killed": t["groups_killed"],
        "cameras_saved": t["groups_saved"],
    })
base = sum_audit(f"{E}/occl_audit_walk_x1.csv")

# ---- probe header: walls / footprints / sensors / groups / query cost -----------------------
probe = open(f"{E}/occl_probe_walk.txt").read()


def grab(pattern, cast=int):
    m = re.search(pattern, probe)
    return cast(m.group(1).replace(",", "")) if m else None


headline = {
    "footprints": grab(r"(\d+) footprints"),
    "walls": grab(r"→ (\d+) walls"),
    "index_build_s": grab(r"indexed in ([\d.]+)s", float),
    "query_us": grab(r"([\d.]+) µs/query", float),
    "sensors_total": grab(r"(\d+) sensors → "),
    "sensors_inside_footprint": grab(r"(\d+) / \d+ sensors \("),
    "camera_groups": grab(r"sensors → (\d+) physical-camera groups"),
    **{k: base[k] for k in ("pairs_free", "pairs_occl", "groups_free", "groups_occl")},
    "cameras_saved_by_another_sightline": base["groups_saved"],
    "cameras_fully_occluded": base["groups_killed"],
    "point_level_attenuation": round(1.0 - base["pairs_occl"] / base["pairs_free"], 6),
    "path_level_attenuation": round(1.0 - base["groups_occl"] / base["groups_free"], 6),
}
if None in headline.values():
    sys.exit(f"occl_probe_walk.txt missing expected lines; parsed {headline}")

# ---- R_i free vs occluded + the disparity deltas ---------------------------------------------


def load(path, key):
    with open(path) as f:
        return {r[key]: r for r in csv.DictReader(f)}


def fnum(d, k):
    v = d.get(k)
    try:
        return float(v) if v not in (None, "") else None
    except ValueError:
        return None


tab = load("data/derived/exposure/exposure_table_nyc.csv", "GEOID")
cov = load("data/derived/exposure/covariates_bg_nyc.csv", "id")
occl = load(f"{E}/R_i_bg_nyc.csv", "id")            # canonical: occluded, walk graph
free = load(f"{E}/R_i_bg_nyc_walk_free.csv", "id")  # occlusion off, walk graph

rows = []
for g, t in tab.items():
    pop = fnum(t, "population")
    if not pop or pop <= 0 or g not in cov:
        continue
    c = cov[g]
    rows.append({
        "pop": pop,
        "free": fnum(free.get(g, {}), "cameras_raw"), "occl": fnum(occl.get(g, {}), "cameras_raw"),
        "pct_hispanic": fnum(t, "pct_hispanic"), "pct_black": fnum(t, "pct_black_nh"),
        "pct_white": fnum(t, "pct_white_nh"), "pct_asian": fnum(t, "pct_asian_nh"),
        "median_hh_income": fnum(t, "median_hh_income"),
        "jobs": fnum(c, "jobs_wsh"), "crime": fnum(c, "crime_wsh"), "req311": fnum(c, "req311_wsh"),
        "transit": fnum(c, "transit_dist_m"), "dens": fnum(c, "pop_wsh"),
        "stations": fnum(c, "stations_wsh"),
    })

pair = [r for r in rows if r["free"] is not None and r["occl"] is not None]
w = np.array([r["pop"] for r in pair], float)
f_v = np.array([r["free"] for r in pair], float)
o_v = np.array([r["occl"] for r in pair], float)
ri_block = {
    "mean_free": float(np.average(f_v, weights=w)),
    "mean_occluded": float(np.average(o_v, weights=w)),
    "pearson_r": float(np.corrcoef(f_v, o_v)[0, 1]),
    "n_block_groups": len(pair),
    "weighting": "population-weighted over populated block groups; zero-exposure BGs included",
}


def wls(y, X, wt):
    n = len(y)
    XtW = X.T * wt
    beta = np.linalg.solve(XtW @ X, XtW @ y)
    e = y - X @ beta
    bread = np.linalg.inv(XtW @ X)
    meat = (X * (wt * e)[:, None]).T @ (X * (wt * e)[:, None])
    covb = bread @ meat @ bread * (n / (n - X.shape[1]))
    return beta, np.sqrt(np.diag(covb))


def rung1(outcome_key, focal):
    sub = [r for r in rows if all(r[k] is not None for k in [outcome_key, focal] + NEED_BASE)]
    wt = np.array([r["pop"] for r in sub], float)
    wt = wt / wt.mean()
    y = np.array([r[outcome_key] for r in sub], float)
    x = np.array([r[focal] for r in sub], float)
    m = np.average(x, weights=wt)
    s = math.sqrt(np.average((x - m) ** 2, weights=wt))
    beta, _ = wls(y, np.column_stack([np.ones(len(sub)), (x - m) / s]), wt)
    return float(beta[1])


disparity = {}
for focal in ["pct_hispanic", "pct_black", "pct_white", "pct_asian", "median_hh_income"]:
    bf, bo = rung1("free", focal), rung1("occl", focal)
    disparity[focal] = {"beta_free": bf, "beta_occluded": bo, "delta": bo - bf}

out = {
    "range_sensitivity": range_sensitivity,
    "headline": headline,
    "R_i": ri_block,
    "disparity": disparity,
    "_note": (
        "Occlusion is a characterized null, now measured on the WALK graph (the audit was "
        "drive-graph-vintage until 2026-07-14). Point-level attenuation (sightlines) vs path-level "
        "(distinct cameras in R_i): streets are the negative space of buildings, so camera-to-street "
        "sightlines run along the canyon, and a camera blocked at one walkshed point usually sees "
        "the walker at another. The effect switches on past ~60 m of assumed range; the x16 arm is "
        "the positive control proving the LOS test is live. disparity betas replicate crime_ladder "
        "rung 1 (pop-weighted WLS, z-scored focal, ladder complete-cases sample). Generated by "
        "tools/occlusion_summary.py from batch occlusion-audit / bg-exposure / occlusion-probe."
    ),
}

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w") as f:
    json.dump(out, f, indent=2)
shutil.copyfile(f"{E}/occl_audit_walk_x1.csv", "data/derived/results/occlusion_audit_bg.csv")

print(f"point-level {100 * headline['point_level_attenuation']:.3f}%  "
      f"path-level {100 * headline['path_level_attenuation']:.3f}%  "
      f"killed {headline['cameras_fully_occluded']}  saved {headline['cameras_saved_by_another_sightline']}")
print(f"R_i mean {ri_block['mean_free']:.3f} -> {ri_block['mean_occluded']:.3f}  "
      f"(r={ri_block['pearson_r']:.6f}, n={ri_block['n_block_groups']})")
for k, v in disparity.items():
    print(f"  {k:18s} beta {v['beta_free']:+.4f} -> {v['beta_occluded']:+.4f}  delta {v['delta']:+.5f}")
print(f"wrote {OUT} (+ results/occlusion_audit_bg.csv)")
