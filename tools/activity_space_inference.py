#!/usr/bin/env python3
"""Clustered and dependence-robust inference for the activity-space regressions.

crime_ladder.py reports HC1 SEs, which assume block groups' errors are independent. Two kinds of
dependence break that for activity-space exposure:

  1. Spatial: neighbouring BGs share walksheds, routes, and neighbourhood unobservables (same
     problem as R_i; see spatial_econometrics.py).
  2. Shared destinations: A_i is 85% a jobs-weighted mean of destination walkshed counts. Origins
     that send workers to the same places (Midtown, Downtown Brooklyn) share those destinations'
     shocks however far apart the origins are. Distance-based Conley kernels miss this.

This script re-runs the crime_ladder specification EXACTLY (same sample, pop weights, z-scoring;
asserted against crime_ladder.json) and, for every focal coefficient, reports:

  hc1          heteroskedasticity-robust (reproduces crime_ladder.json)
  cr1_<unit>   cluster-robust (CR1) by origin tract, NTA 2020, PUMA 2020
  wcb_<unit>   wild cluster bootstrap-t p-value, null imposed (WCR), Webb 6-point weights; the
               reliable test at few clusters (NTA ~195, PUMA 55)
  conley_<bw>  spatial HAC, triangular kernel on BG centroid distance (2 km, 5 km)

None of these is valid under dependence (2). tools/inference_montecarlo.py simulates errors built
from shared destination shocks on the real LODES shares: every estimator here, PUMA clustering
included, recovers well under half the true sampling SD. A destination-overlap HAC (kernel =
cosine similarity of destination share vectors) was tried and dropped: it is biased low even under
iid errors (wide-kernel HAC subtracts variance because OLS residuals are orthogonal to X). So the
clustered SEs are LOWER BOUNDS for A_i; valid inference for A_i needs the design-based
random-destination null (docs/RELATED_METHODS.md recommendation 10).

Writes data/derived/results/activity_space_inference.json.
"""
import csv, json, math, os, sys

import numpy as np
from scipy.spatial import cKDTree

OUTDIR = "data/derived/results"
PUMA_REL = "data/snapshots/geo/tract_to_puma_2020.txt"
NTA_GEO = "data/snapshots/geo/nta2020.geojson"
CENTROIDS = "data/snapshots/census/bg_centroids_nyc.csv"
CONLEY_BWS = (2000.0, 5000.0)
B_BOOT = int(os.environ.get("OURSPACE_WCB_B", "9999"))
SEED = 20260923

CONTROLS = ["jobs", "transit", "dens", "stations"]  # identical to crime_ladder.py
NEED_BASE = CONTROLS + ["crime", "req311"]
OUTCOMES = ["A_mnl", "E_i", "R_i"]
FOCALS = [("pct_black", "%Black"), ("pct_hisp", "%Hisp"), ("pct_white", "%White"), ("income", "income")]
RUNGS = ["1 total (demographic only)", "2 +land-use", "3 +crime", "4 +crime+311"]
RUNG_KEYS = ["rung1_total", "rung2_landuse", "rung3_crime", "rung4_crime_311"]


def load(path, key):
    with open(path) as f:
        return {r[key]: r for r in csv.DictReader(f)}


def fnum(d, k):
    v = d.get(k)
    try:
        return float(v) if v not in (None, "") else None
    except ValueError:
        return None


# ---------------------------------------------------------------- data (as crime_ladder.py) ---
tab = load("data/derived/exposure/exposure_table_nyc.csv", "GEOID")
cov = load("data/derived/exposure/covariates_bg_nyc.csv", "id")
rows = []
for g in tab:
    if g not in cov:
        continue
    t, c = tab[g], cov[g]
    pop = fnum(t, "population")
    if not pop or pop <= 0:
        continue
    rows.append({
        "geoid": g, "pop": pop,
        "R_i": fnum(t, "R_i"), "E_i": fnum(t, "E_i"), "A_mnl": fnum(t, "A_mnl"),
        "pct_black": fnum(t, "pct_black_nh"), "pct_hisp": fnum(t, "pct_hispanic"),
        "pct_white": fnum(t, "pct_white_nh"), "income": fnum(t, "median_hh_income"),
        "jobs": fnum(c, "jobs_wsh"), "crime": fnum(c, "crime_wsh"), "req311": fnum(c, "req311_wsh"),
        "transit": fnum(c, "transit_dist_m"), "dens": fnum(c, "pop_wsh"),
        "stations": fnum(c, "stations_wsh"),
    })


def sample(keys):
    sub = [r for r in rows if all(r[k] is not None for k in keys)]
    w = np.array([r["pop"] for r in sub], float)
    return sub, w / w.mean()


# ------------------------------------------------------------------------ cluster crosswalks ---
tract_puma = {}
with open(PUMA_REL, encoding="utf-8-sig") as f:
    for r in csv.DictReader(f):
        if r["STATEFP"] == "36":
            tract_puma[r["STATEFP"] + r["COUNTYFP"] + r["TRACTCE"]] = r["PUMA5CE"]

cent = {}
with open(CENTROIDS) as f:
    for r in csv.DictReader(f):
        cent[r["id"]] = (float(r["lat"]), float(r["lon"]))


def nta_lookup():
    """BG centroid -> NTA 2020 code by point-in-polygon (NTAs are unions of 2020 tracts)."""
    import geopandas as gpd
    from shapely.geometry import Point
    nta = gpd.read_file(NTA_GEO)[["nta2020", "geometry"]].to_crs(4326)
    ids = list(cent)
    pts = gpd.GeoDataFrame({"geoid": ids}, geometry=[Point(cent[i][1], cent[i][0]) for i in ids], crs=4326)
    j = gpd.sjoin(pts, nta, how="left", predicate="within")
    out = dict(zip(j["geoid"], j["nta2020"]))
    # centroids falling in water/slivers: nearest NTA
    miss = [i for i in ids if not isinstance(out.get(i), str)]
    if miss:
        mp = pts[pts["geoid"].isin(miss)].to_crs(32618)
        nn = gpd.sjoin_nearest(mp, nta.to_crs(32618), how="left")
        out.update(dict(zip(nn["geoid"], nn["nta2020"])))
    return out


bg_nta = nta_lookup()

# ----------------------------------------------------------------------------- estimators ---
def fit(y, X, w):
    XtW = X.T * w
    bread = np.linalg.inv(XtW @ X)
    beta = bread @ (XtW @ y)
    e = y - X @ beta
    S = X * (w * e)[:, None]           # score contributions s_i = w_i x_i e_i
    return beta, e, bread, S


def v_from_meat(bread, meat, dof=1.0):
    return bread @ meat @ bread * dof


def cr1(bread, S, groups, n, k):
    labels, inv = np.unique(groups, return_inverse=True)
    G = len(labels)
    Sg = np.zeros((G, S.shape[1]))
    np.add.at(Sg, inv, S)
    dof = (G / (G - 1)) * ((n - 1) / (n - k))
    return v_from_meat(bread, Sg.T @ Sg, dof), G


def conley_kernel(xy, bw):
    """Sparse triangular kernel on centroid distance; the diagonal (d=0) enters with weight 1."""
    from scipy.sparse import coo_matrix
    tree = cKDTree(xy)
    D = tree.sparse_distance_matrix(tree, bw, output_type="ndarray")
    off = D["i"] != D["j"]             # self pairs added explicitly below (and co-located BGs,
    i, j, d = D["i"][off], D["j"][off], D["v"][off]   # d=0, keep weight 1 via 1 - 0/bw)
    n = len(xy)
    rows = np.concatenate([i, np.arange(n)]); cols = np.concatenate([j, np.arange(n)])
    vals = np.concatenate([1.0 - d / bw, np.ones(n)])
    return coo_matrix((vals, (rows, cols)), shape=(n, n)).tocsr()


def kernel_v(bread, S, K):
    meat = S.T @ (K @ S)
    return v_from_meat(bread, (meat + meat.T) / 2)


def wild_cluster_p(y, X, w, groups, j, rng, B=B_BOOT):
    """WCR bootstrap-t for H0: beta_j = 0 with CR1 SEs; Webb 6-point weights by cluster."""
    sw = np.sqrt(w)
    Xs, ys = X * sw[:, None], y * sw
    n, k = Xs.shape
    labels, inv = np.unique(groups, return_inverse=True)
    G = len(labels)
    dof = (G / (G - 1)) * ((n - 1) / (n - k))
    XtXi = np.linalg.inv(Xs.T @ Xs)
    A = XtXi @ Xs.T                       # beta = A y
    q = A[j]                              # focal row: beta_j = q . y ; score weights for SE_j

    def t_stats(Y):                       # Y: n x b
        bj = q @ Y
        U = Y - Xs @ (A @ Y)
        C = np.zeros((G, Y.shape[1]))
        np.add.at(C, inv, q[:, None] * U)
        return bj / np.sqrt(dof * (C ** 2).sum(0))

    t_obs = float(t_stats(ys[:, None])[0])
    Xr = np.delete(Xs, j, axis=1)         # restricted fit (null imposed)
    br = np.linalg.lstsq(Xr, ys, rcond=None)[0]
    fitted_r, ur = Xr @ br, ys - Xr @ br
    webb = np.array([-math.sqrt(1.5), -1.0, -math.sqrt(0.5), math.sqrt(0.5), 1.0, math.sqrt(1.5)])
    exceed, done = 0, 0
    while done < B:
        b = min(1000, B - done)
        V = webb[rng.integers(0, 6, size=(G, b))]
        Yb = fitted_r[:, None] + ur[:, None] * V[inv]
        exceed += int((np.abs(t_stats(Yb)) >= abs(t_obs)).sum())
        done += b
    return (exceed + 1) / (B + 1), t_obs, G


# ------------------------------------------------------------------------------------ run ---
def main():
    ref = json.load(open(os.path.join(OUTDIR, "crime_ladder.json")))["ladders"]
    rng = np.random.default_rng(SEED)
    out = {"_readme": {
        "spec": "crime_ladder.py ladders re-run verbatim (asserted); coefficient + SE per estimator",
        "clusters": {"tract": "GEOID[:11]", "nta": "NYC DCP NTA 2020 (point-in-polygon of BG centroid)",
                     "puma": "Census 2020 tract->PUMA relationship file"},
        "conley": "triangular kernel on centroid distance, bandwidths (m) %s" % list(CONLEY_BWS),
        "caveat": "all SEs understate uncertainty under shared-destination dependence "
                  "(see tools/inference_montecarlo.py); treat them as lower bounds for A_mnl and E_i",
        "wcb": f"WCR bootstrap-t, Webb weights, B={B_BOOT}, seed={SEED}; reported for rungs 1 and 4",
    }, "ladders": {}}

    for yname in OUTCOMES:
        print(f"\n########## {yname}: focal coefficient (cameras/SD) and SEs by estimator ##########")
        print(f"  {'focal':7s} {'rung':4s} {'beta':>7s} {'HC1':>6s} {'tract':>6s} {'NTA':>6s} {'PUMA':>6s}"
              f" {'C2km':>6s} {'C5km':>6s}  {'wcbNTA':>7s} {'wcbPUMA':>7s}")
        for fk, fn in FOCALS:
            sub, w = sample([yname, fk] + NEED_BASE)
            n = len(sub)
            geoids = [r["geoid"] for r in sub]
            col = lambda k, log=False: (np.log1p if log else np.asarray)(np.array([r[k] for r in sub], float))

            def z(x):
                m = np.average(x, weights=w)
                s = math.sqrt(np.average((x - m) ** 2, weights=w))
                return (x - m) / s if s > 0 else x * 0

            y, ones, zf = col(yname), np.ones(n), z(col(fk))
            base = [zf] + [z(col(k, 1)) for k in CONTROLS]
            designs = [[zf], base, base + [z(col("crime", 1))], base + [z(col("crime", 1)), z(col("req311", 1))]]
            tracts = np.array([g[:11] for g in geoids])
            pumas = np.array([tract_puma.get(g[:11], "?") for g in geoids])
            ntas = np.array([bg_nta.get(g, "?") for g in geoids])
            assert "?" not in set(pumas) and "?" not in set(ntas), "unmatched cluster ids"
            lat = np.array([cent[g][0] for g in geoids]); lon = np.array([cent[g][1] for g in geoids])
            xy = np.column_stack([lon * 111_320 * math.cos(math.radians(40.7)), lat * 110_540])
            KC = {bw: conley_kernel(xy, bw) for bw in CONLEY_BWS}

            recs = {}
            for rung, rk, terms in zip(RUNGS, RUNG_KEYS, designs):
                X = np.column_stack([ones] + terms)
                k = X.shape[1]
                beta, e, bread, S = fit(y, X, w)
                se = lambda V: float(math.sqrt(max(V[1, 1], 0.0)))
                hc1 = se(v_from_meat(bread, S.T @ S, n / (n - k)))
                b_ref, se_ref = ref[yname][fn][rk]
                assert abs(beta[1] - b_ref) < 1e-8 and abs(hc1 - se_ref) < 1e-8, (yname, fn, rk)
                r = {"n": n, "beta": float(beta[1]), "hc1": hc1}
                for lbl, grp in (("tract", tracts), ("nta", ntas), ("puma", pumas)):
                    V, G = cr1(bread, S, grp, n, k)
                    r[f"cr1_{lbl}"], r[f"G_{lbl}"] = se(V), G
                for bw in CONLEY_BWS:
                    r[f"conley_{bw/1000:g}km"] = se(kernel_v(bread, S, KC[bw]))
                if rk in ("rung1_total", "rung4_crime_311"):
                    for lbl, grp in (("nta", ntas), ("puma", pumas)):
                        p, t, G = wild_cluster_p(y, X, w, grp, 1, rng)
                        r[f"wcb_{lbl}_p"], r[f"wcb_{lbl}_t"] = p, t
                recs[rk] = r
                wn = f"{r['wcb_nta_p']:7.4f}" if "wcb_nta_p" in r else "       "
                wp = f"{r['wcb_puma_p']:7.4f}" if "wcb_puma_p" in r else "       "
                print(f"  {fn:7s} r{rung[0]:3s} {r['beta']:+7.2f} {hc1:6.2f} {r['cr1_tract']:6.2f} {r['cr1_nta']:6.2f}"
                      f" {r['cr1_puma']:6.2f} {r['conley_2km']:6.2f} {r['conley_5km']:6.2f}  {wn} {wp}")
            out["ladders"].setdefault(yname, {})[fn] = recs
            sys.stdout.flush()

    g0 = out["ladders"]["A_mnl"]["%Hisp"]["rung1_total"]
    print(f"\nclusters: tract {g0['G_tract']}, NTA {g0['G_nta']}, PUMA {g0['G_puma']}. SEs are lower bounds "
          "for A_mnl/E_i under shared-destination dependence (tools/inference_montecarlo.py).")
    os.makedirs(OUTDIR, exist_ok=True)
    with open(os.path.join(OUTDIR, "activity_space_inference.json"), "w") as f:
        json.dump(out, f, indent=2)
    print(f"wrote {OUTDIR}/activity_space_inference.json")


if __name__ == "__main__":
    main()
