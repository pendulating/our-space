#!/usr/bin/env python3
"""Random-destination null model for the activity-space measure (design-based inference).

The NEAP literature (Kwan 2018; Cai & Kwan 2024) predicts that averaging exposure over many
destinations shrinks dispersion mechanically, and the shared-destination structure of A_i
defeats every analytic SE (tools/inference_montecarlo.py). The design-based answer is to hold
everything fixed except the one thing the paper is about — WHICH destinations a block group's
workers go to — and re-run the real instrument on random reassignments.

Null (default, `--mode jobs`): each worker draws a destination from the CITYWIDE job
distribution, independent of home. This keeps how many people commute from each block group
(the LODES home marginal) and where the jobs are (the LODES work marginal), and removes only
the home->work pairing. `--mode bgs` draws destinations uniformly over block groups instead
(also erases the job geography; kept as a switch, not the headline).

    make    write data/derived/null/od_null_<d>.csv, same schema as bg_od_nyc.csv, one per
            draw (seeded); then `sbatch tools/cluster_null_od.sh` routes each with the real
            `batch od-exposure-mnl`.
    analyze compare the observed A_mnl against the null draws: pop-weighted Gini / top-decile
            share / P90:P10 of A, A_dest, commute, A+R, A_lived; income-quintile means; the
            crime_ladder rung-1 coefficient of A on each focal demographic. Monte Carlo p-values
            are (1 + #{null at least as extreme}) / (N + 1).

Writes data/derived/results/null_destinations.json.
"""
import argparse, csv, glob, json, math, os, sys
from collections import defaultdict

import numpy as np

OD = "data/snapshots/lodes/bg_od_nyc.csv"
NULL_DIR = "data/derived/null"
EXP = "data/derived/exposure"
ACS = "data/snapshots/census/acs_nyc.csv"
OUT = "data/derived/results/null_destinations.json"
SEED = 20260924
BORO = {"36005": "Bronx", "36047": "Brooklyn", "36061": "Manhattan", "36081": "Queens", "36085": "Staten Island"}


# ------------------------------------------------------------------------------- make ---
def make(n_draws, mode, start=0):
    homes = defaultdict(float)
    works = defaultdict(float)
    with open(OD) as f:
        for r in csv.DictReader(f):
            j = float(r["jobs"])
            homes[r["home_bg"]] += j
            works[r["work_bg"]] += j
    work_ids = sorted(works)
    if mode == "jobs":
        q = np.array([works[w] for w in work_ids]); q /= q.sum()
    else:
        q = np.full(len(work_ids), 1.0 / len(work_ids))
    os.makedirs(NULL_DIR, exist_ok=True)
    total_jobs = sum(homes.values())
    for d in range(start, start + n_draws):
        rng = np.random.default_rng(SEED + d)
        path = f"{NULL_DIR}/od_null_{mode}_{d:03d}.csv"
        n_rows = 0
        with open(path, "w") as f:
            f.write("home_bg,work_bg,jobs,low_wage\n")
            for h in sorted(homes):
                J = int(round(homes[h]))
                if J <= 0:
                    continue
                counts = rng.multinomial(J, q)
                for k in np.flatnonzero(counts):
                    f.write(f"{h},{work_ids[k]},{int(counts[k])},0\n")
                    n_rows += 1
        print(f"  draw {d:03d}: {n_rows:,} pairs, {total_jobs:,.0f} jobs -> {path}", flush=True)


# ---------------------------------------------------------------------------- analyze ---
def wavg(x, w):
    return float(np.average(x, weights=w))


def gini(x, w):
    o = np.argsort(x); x = np.asarray(x, float)[o]; w = np.asarray(w, float)[o]
    cw = np.cumsum(w) / w.sum(); cx = np.cumsum(x * w) / (x * w).sum()
    return float(1 - np.sum((cx[1:] + cx[:-1]) * np.diff(cw)) - cx[0] * cw[0])


def wquantile(x, w, q):
    o = np.argsort(x); x = np.asarray(x, float)[o]; w = np.asarray(w, float)[o]
    cw = (np.cumsum(w) - 0.5 * w) / w.sum()
    return float(np.interp(q, cw, x))


def top_decile_share(x, w):
    o = np.argsort(x); x = np.asarray(x, float)[o]; w = np.asarray(w, float)[o]
    cw = np.cumsum(w) / w.sum()
    top = cw >= 0.9
    return float((x[top] * w[top]).sum() / (x * w).sum())


def wls_focal(y, focal, w):
    """crime_ladder rung 1: pop-weighted WLS of y on the pop-z-scored focal; returns beta (cameras/SD)."""
    m = wavg(focal, w); s = math.sqrt(wavg((focal - m) ** 2, w))
    z = (focal - m) / s if s > 0 else focal * 0
    X = np.column_stack([np.ones(len(y)), z])
    XtW = X.T * w
    return float(np.linalg.solve(XtW @ X, XtW @ y)[1])


def load_frame():
    import pandas as pd
    t = pd.read_csv(f"{EXP}/exposure_table_nyc.csv", dtype={"GEOID": str})
    t = t[t.population > 0].copy()
    acs = pd.read_csv(ACS, dtype={"id": str})
    acs["c"] = ((acs.commute_total - acs.commute_wfh) / acs.pop_total).clip(0, 1)
    t = t.merge(acs[["id", "c"]], left_on="GEOID", right_on="id", how="left")
    a = pd.read_csv(f"{EXP}/A_i_mnl_bg_nyc.csv", dtype={"home_bg": str})[["home_bg", "A_dest", "A_modal"]]
    a = a.rename(columns={"A_dest": "Adest_obs", "A_modal": "A_obs"})
    t = t.merge(a, left_on="GEOID", right_on="home_bg", how="inner")
    assert np.allclose(t.A_obs, t.A_mnl), "A_i_mnl file and exposure table disagree"
    return t


def stats(t, A, Adest):
    """All statistics for one A vector (aligned to t)."""
    w = t.population.values
    R = t.R_i.values
    commute = A - Adest
    c = t.c.fillna(0).values
    lived = c * A + (1 - c) * R
    out = {}
    for name, x in (("A", A), ("A_dest", Adest), ("commute", commute), ("A_plus_R", A + R), ("A_lived", lived)):
        out[f"gini_{name}"] = gini(x, w)
        out[f"top10_{name}"] = top_decile_share(x, w)
        out[f"mean_{name}"] = wavg(x, w)
    out["p90_p10_A"] = wquantile(A, w, 0.9) / wquantile(A, w, 0.1)
    # income quintiles (pop-weighted, BGs with income)
    inc = t.median_hh_income.values
    ok = ~np.isnan(inc)
    edges = [wquantile(inc[ok], w[ok], q) for q in (0.2, 0.4, 0.6, 0.8)]
    qi = np.digitize(inc[ok], edges)
    out["A_by_income_quintile"] = [wavg(A[ok][qi == k], w[ok][qi == k]) for k in range(5)]
    out["A_Q5_minus_Q1"] = out["A_by_income_quintile"][4] - out["A_by_income_quintile"][0]
    # rung-1 gradients (complete cases on the focal; population weights)
    for key, col in (("pct_hisp", "pct_hispanic"), ("pct_black", "pct_black_nh"), ("pct_white", "pct_white_nh"), ("income", "median_hh_income")):
        f = t[col].values.astype(float)
        m = ~np.isnan(f)
        out[f"beta_{key}"] = wls_focal(A[m], f[m], w[m])
    # by borough
    boro = t.GEOID.str[:5].map(BORO).values
    out["A_by_borough"] = {b: wavg(A[boro == b], w[boro == b]) for b in sorted(BORO.values())}
    return out


def analyze(mode):
    import pandas as pd
    t = load_frame()
    obs = stats(t, t.A_obs.values, t.Adest_obs.values)
    files = sorted(glob.glob(f"{NULL_DIR}/A_i_mnl_null_{mode}_*.csv"))
    if not files:
        raise SystemExit(f"no null A files in {NULL_DIR} (run make + cluster_null_od.sh first)")
    draws = []
    for fp in files:
        a = pd.read_csv(fp, dtype={"home_bg": str})[["home_bg", "A_dest", "A_modal"]]
        m = t[["GEOID"]].merge(a, left_on="GEOID", right_on="home_bg", how="left")
        if m.A_modal.isna().any():
            miss = int(m.A_modal.isna().sum())
            print(f"  {os.path.basename(fp)}: {miss} BGs unrouted in the null draw; filled with observed", file=sys.stderr)
            m["A_modal"] = m.A_modal.fillna(pd.Series(t.A_obs.values)); m["A_dest"] = m.A_dest.fillna(pd.Series(t.Adest_obs.values))
        draws.append(stats(t, m.A_modal.values, m.A_dest.values))
    N = len(draws)
    scalar_keys = [k for k, v in obs.items() if isinstance(v, float)]
    summary = {}
    for k in scalar_keys:
        arr = np.array([d[k] for d in draws])
        # Design-based test: is the observed value extreme relative to the null distribution?
        # Two-sided around the null mean for everything; dispersion measures additionally get
        # the one-sided P(null >= observed) that answers "is the observed A MORE unequal than
        # commuting-without-geography would make it".
        mu = arr.mean()
        p2 = (1 + int((np.abs(arr - mu) >= abs(obs[k] - mu)).sum())) / (N + 1)
        rec = {"observed": obs[k], "null_mean": float(mu), "null_sd": float(arr.std(ddof=1)) if N > 1 else None,
               "null_min": float(arr.min()), "null_max": float(arr.max()),
               "z": float((obs[k] - mu) / arr.std(ddof=1)) if N > 1 and arr.std(ddof=1) > 0 else None,
               "mc_p_two_sided": p2, "outside_null_range": bool(obs[k] > arr.max() or obs[k] < arr.min())}
        if k.startswith(("gini_", "top10_", "p90_p10")):
            rec["mc_p_null_ge_observed"] = (1 + int((arr >= obs[k]).sum())) / (N + 1)
        summary[k] = rec
    summary["A_by_income_quintile"] = {"observed": obs["A_by_income_quintile"],
                                       "null_mean": np.mean([d["A_by_income_quintile"] for d in draws], axis=0).tolist()}
    summary["A_by_borough"] = {"observed": obs["A_by_borough"],
                               "null_mean": {b: float(np.mean([d["A_by_borough"][b] for d in draws])) for b in obs["A_by_borough"]}}
    out = {"_readme": {
        "null": f"destinations redrawn per worker from the citywide {'job' if mode == 'jobs' else 'uniform-over-BG'} distribution, independent of home; "
                "LODES home marginal (commuters per BG) preserved; the real batch od-exposure-mnl routed every draw",
        "n_draws": N, "seed_base": SEED, "files": [os.path.basename(f) for f in files],
        "gini_R_i_for_reference": gini(t.R_i.values, t.population.values),
        "mc_p_two_sided": "(1 + #{|null - null_mean| >= |observed - null_mean|}) / (N + 1); resolution 1/(N+1)",
        "mc_p_null_ge_observed": "dispersion measures only: (1 + #{null >= observed}) / (N + 1)",
    }, "stats": summary}
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with open(OUT, "w") as f:
        json.dump(out, f, indent=2)

    print(f"random-destination null ({mode}), {N} draws")
    print(f"  {'statistic':22s} {'observed':>9s} {'null mean':>10s} {'null sd':>8s} {'z':>7s} {'p(2s)':>6s} {'outside':>7s}")
    for k in ["gini_A", "gini_A_dest", "gini_commute", "gini_A_plus_R", "gini_A_lived", "top10_A", "p90_p10_A",
              "mean_A", "A_Q5_minus_Q1", "beta_pct_hisp", "beta_pct_black", "beta_pct_white", "beta_income"]:
        s = summary[k]
        z = f"{s['z']:7.1f}" if s["z"] is not None else "    n/a"
        sd = f"{s['null_sd']:8.4f}" if s["null_sd"] is not None else "     n/a"
        print(f"  {k:22s} {s['observed']:9.4f} {s['null_mean']:10.4f} {sd} {z} {s['mc_p_two_sided']:6.3f} {str(s['outside_null_range']):>7s}")
    print(f"  Gini(R_i) = {out['_readme']['gini_R_i_for_reference']:.4f} for reference")
    print(f"  -> {OUT}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("cmd", choices=["make", "analyze"])
    ap.add_argument("--draws", type=int, default=20)
    ap.add_argument("--start", type=int, default=0)
    ap.add_argument("--mode", choices=["jobs", "bgs"], default="jobs")
    a = ap.parse_args()
    make(a.draws, a.mode, a.start) if a.cmd == "make" else analyze(a.mode)
