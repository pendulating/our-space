#!/usr/bin/env python3
"""The walk-vs-drive network correction, persisted (§Results robustness, the networks paragraph).

Every walkshed in the paper runs on the pedestrian graph (graph_nyc_walk.osgraph). This script
persists the BEFORE/AFTER of that correction so the paper's "the correction raises mean R_i by
X% and the %Hispanic coefficient by Y" sentence is generated, not typed. Until 2026-07-14 those
two numbers were hardcoded in 04_results.tex from a superseded intermediate run (+10.19 -> +10.57,
the pre-complete-cases N=5,547 chain); the canonical pair is +9.47 -> +9.84.

Inputs (the drive arm is a bake-level artifact; regenerate with):
    target/release/batch bg-exposure crates/app-interactive/assets/processed/graph_nyc.osgraph \
        data/snapshots/census/bg_centroids_nyc.csv data/derived/exposure/R_i_bg_nyc_drive.csv 10
    (occlusion on, i.e. the default -- identical settings to the canonical walk run; the ONLY
     difference between the arms is the street network.)

The %Hispanic coefficient replicates crime_ladder.py's rung 1 EXACTLY (population-weighted WLS,
HC1, focal z-scored, complete cases on outcome + focal + the full control set, held fixed), and
asserts that the walk arm reproduces crime_ladder.json's rung1_total before writing anything.

Writes data/derived/results/network_effect.json
"""
import csv, json, math, os, sys

import numpy as np

OUT = "data/derived/results/network_effect.json"
WALK_CSV = "data/derived/exposure/R_i_bg_nyc.csv"          # canonical (occluded, walk graph)
DRIVE_CSV = "data/derived/exposure/R_i_bg_nyc_drive.csv"   # same settings, drive graph
NEED_BASE = ["jobs", "transit", "dens", "stations", "crime", "req311"]  # crime_ladder's sample rule


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
arms = {"walk": load(WALK_CSV, "id"), "drive": load(DRIVE_CSV, "id")}

rows = []
for g, t in tab.items():
    pop = fnum(t, "population")
    if not pop or pop <= 0 or g not in cov:
        continue
    c = cov[g]
    rows.append({
        "pop": pop,
        "walk": fnum(arms["walk"].get(g, {}), "cameras_raw"),
        "drive": fnum(arms["drive"].get(g, {}), "cameras_raw"),
        "pct_hisp": fnum(t, "pct_hispanic"),
        "jobs": fnum(c, "jobs_wsh"), "crime": fnum(c, "crime_wsh"), "req311": fnum(c, "req311_wsh"),
        "transit": fnum(c, "transit_dist_m"), "dens": fnum(c, "pop_wsh"),
        "stations": fnum(c, "stations_wsh"),
    })


def wls(y, X, w):
    n = len(y)
    XtW = X.T * w
    beta = np.linalg.solve(XtW @ X, XtW @ y)
    e = y - X @ beta
    bread = np.linalg.inv(XtW @ X)
    meat = (X * (w * e)[:, None]).T @ (X * (w * e)[:, None])
    covb = bread @ meat @ bread * (n / (n - X.shape[1]))
    return beta, np.sqrt(np.diag(covb))


def rung1(outcome_key):
    """crime_ladder.py rung 1, verbatim: bivariate on z(focal), ladder complete-cases sample."""
    sub = [r for r in rows if all(r[k] is not None for k in [outcome_key, "pct_hisp"] + NEED_BASE)]
    w = np.array([r["pop"] for r in sub], float)
    w = w / w.mean()
    y = np.array([r[outcome_key] for r in sub], float)
    x = np.array([r["pct_hisp"] for r in sub], float)
    m = np.average(x, weights=w)
    s = math.sqrt(np.average((x - m) ** 2, weights=w))
    zf = (x - m) / s
    beta, se = wls(y, np.column_stack([np.ones(len(sub)), zf]), w)
    return float(beta[1]), float(se[1]), len(sub)


def mean_popwtd(outcome_key):
    """Pop-weighted mean over pop>0 rows; zeros are real data and stay in (inequality_stats convention)."""
    sub = [r for r in rows if r[outcome_key] is not None]
    return float(np.average([r[outcome_key] for r in sub], weights=[r["pop"] for r in sub])), len(sub)


res = {}
for arm in ("walk", "drive"):
    b, se_, n = rung1(arm)
    mu, n_mu = mean_popwtd(arm)
    res[arm] = {"mean_popwtd": mu, "n_mean": n_mu, "hisp_rung1": [b, se_], "n_ladder": n}

# The walk arm must reproduce the canonical ladder before anything downstream trusts this file.
lad = json.load(open("data/derived/results/crime_ladder.json"))["ladders"]["R_i"]["%Hisp"]
drift = abs(res["walk"]["hisp_rung1"][0] - lad["rung1_total"][0])
if drift > 5e-3:
    sys.exit(
        f"network_effect: walk-arm rung1 {res['walk']['hisp_rung1'][0]:+.4f} does not reproduce "
        f"crime_ladder.json's {lad['rung1_total'][0]:+.4f} (drift {drift:.4f}). The exposure CSVs "
        f"and crime_ladder.json are out of sync -- re-run the chain before trusting this artifact."
    )

res["delta"] = {
    "mean_pct": 100.0 * (res["walk"]["mean_popwtd"] / res["drive"]["mean_popwtd"] - 1.0),
    "hisp": res["walk"]["hisp_rung1"][0] - res["drive"]["hisp_rung1"][0],
}
res["note"] = (
    "walk = canonical R_i (pedestrian graph, occlusion on); drive = identical bake on the drive "
    "graph. hisp_rung1 replicates crime_ladder.py rung 1 (pop-weighted WLS, HC1, z-scored focal, "
    "complete cases incl. the full control set). The walk arm is asserted equal to "
    "crime_ladder.json at write time."
)

os.makedirs(os.path.dirname(OUT), exist_ok=True)
with open(OUT, "w") as f:
    json.dump(res, f, indent=2)

print(f"walk : mean {res['walk']['mean_popwtd']:.2f}  %Hisp rung1 {res['walk']['hisp_rung1'][0]:+.3f} "
      f"(±{res['walk']['hisp_rung1'][1]:.3f})  N={res['walk']['n_ladder']}")
print(f"drive: mean {res['drive']['mean_popwtd']:.2f}  %Hisp rung1 {res['drive']['hisp_rung1'][0]:+.3f} "
      f"(±{res['drive']['hisp_rung1'][1]:.3f})  N={res['drive']['n_ladder']}")
print(f"delta: mean {res['delta']['mean_pct']:+.2f}%   %Hisp {res['delta']['hisp']:+.3f} cameras/SD")
print(f"wrote {OUT}")
