#!/usr/bin/env python3
"""Population-honest lived exposure: mix commuters and non-commuters.

`A_mnl` routes LODES *workers* through their commute; population-weighted statistics then
quietly generalize it to everyone. But a non-worker's day has no commute, and neither does
a work-from-home worker's (ACS B08301_021): their lived exposure is essentially the
residential walkshed `R_i` -- the UNEQUAL measure. This script computes the honest mixture

    c_i       = (commute_total_i - commute_wfh_i) / pop_i      (commuter share, clamped)
    A_lived_i = c_i * A_mnl_i + (1 - c_i) * R_i

and reports whether the equalization headline survives population accounting: Gini,
population-weighted demographic correlations, and income-quintile means of A_lived
alongside R_i and A_mnl. Children/retirees/unemployed are assigned the R_i day (school
and non-work travel are out of scope, disclosed).

In : data/derived/exposure/R_i_bg_nyc.csv          (cameras_corrected)
     data/derived/exposure/A_i_mnl_bg_nyc.csv      (A_modal column = A_mnl)
     data/snapshots/census/bg_centroids_nyc.csv    (population weights)
     data/snapshots/census/acs_nyc.csv             (commute_total, commute_wfh, demographics)
Out: data/derived/results/population_mixed.json
"""
import csv
import json
import math
import sys

import numpy as np

E = "data/derived/exposure"


def load(path, key="id"):
    with open(path) as f:
        return {r[key]: r for r in csv.DictReader(f)}


def fnum(v):
    try:
        x = float(v)
        return x if math.isfinite(x) else None
    except (TypeError, ValueError):
        return None


def gini(x, w):
    # Identical estimator to inequality_stats.py (population-ordered Lorenz, trapezoid),
    # so A_lived is comparable with the canonical Gini rows to the last digit.
    x, w = np.asarray(x, float), np.asarray(w, float)
    o = np.argsort(x, kind="mergesort")
    x, w = x[o], w[o]
    p = w / w.sum()
    e = p * x
    if e.sum() <= 0:
        return 0.0
    P = np.concatenate([[0.0], np.cumsum(p)])
    L = np.concatenate([[0.0], np.cumsum(e) / e.sum()])
    return float(1.0 - np.sum(np.diff(P) * (L[:-1] + L[1:])))


def wcorr(x, y, w):
    x, y, w = np.asarray(x, float), np.asarray(y, float), np.asarray(w, float)
    mx, my = np.average(x, weights=w), np.average(y, weights=w)
    cov = np.average((x - mx) * (y - my), weights=w)
    sx = math.sqrt(np.average((x - mx) ** 2, weights=w))
    sy = math.sqrt(np.average((y - my) ** 2, weights=w))
    return float(cov / (sx * sy)) if sx > 0 and sy > 0 else 0.0


def main() -> int:
    ri = load(f"{E}/R_i_bg_nyc.csv")
    am = load(f"{E}/A_i_mnl_bg_nyc.csv", key="home_bg")
    acs = load("data/snapshots/census/acs_nyc.csv")
    pop = {}
    with open("data/snapshots/census/bg_centroids_nyc.csv") as f:
        for line in f:
            t = line.strip().split(",")
            if len(t) >= 4 and fnum(t[3]) is not None:
                pop[t[0]] = float(t[3])

    rows = []
    c_missing = 0
    for g, r in ri.items():
        R = fnum(r.get("cameras_corrected"))
        p = pop.get(g, 0.0)
        a = am.get(g)
        A = fnum(a.get("A_modal")) if a else None
        if R is None or p <= 0:
            continue
        d = acs.get(g, {})
        tot, wfh = fnum(d.get("commute_total")), fnum(d.get("commute_wfh"))
        if tot is not None:
            c = max(0.0, min(1.0, (tot - (wfh or 0.0)) / p))
        else:
            c = None
            c_missing += 1
        rows.append(
            {
                "g": g,
                "pop": p,
                "R": R,
                # BGs with no LODES home flows (unrouted) have no commuter day at all:
                # everyone there lives the R_i day (c forced to 0 by A=None below).
                "A": A,
                "c": c,
                "income": fnum(d.get("median_hh_income")),
                "hisp": fnum(d.get("hispanic")),
                "black": fnum(d.get("black_nh")),
                "ptot": fnum(d.get("pop_total")),
            }
        )
    c_mean = float(
        np.average(
            [r["c"] for r in rows if r["c"] is not None],
            weights=[r["pop"] for r in rows if r["c"] is not None],
        )
    )
    for r in rows:
        c = r["c"] if r["c"] is not None else c_mean
        if r["A"] is None:
            c = 0.0
        r["cc"] = c
        r["A_lived"] = c * (r["A"] or 0.0) + (1.0 - c) * r["R"]

    w = [r["pop"] for r in rows]
    g_R = gini([r["R"] for r in rows], w)
    g_lived = gini([r["A_lived"] for r in rows], w)
    routed = [r for r in rows if r["A"] is not None]
    wr = [r["pop"] for r in routed]
    g_A = gini([r["A"] for r in routed], wr)

    def corrs(key):
        s = [r for r in rows if r["income"] is not None and r["ptot"]]
        return {
            "income": wcorr([r[key] for r in s], [r["income"] for r in s], [r["pop"] for r in s]),
            "pct_hispanic": wcorr(
                [r[key] for r in s], [r["hisp"] / r["ptot"] for r in s], [r["pop"] for r in s]
            ),
            "pct_black": wcorr(
                [r[key] for r in s], [r["black"] / r["ptot"] for r in s], [r["pop"] for r in s]
            ),
        }

    # Income-quintile means of A_lived (population-weighted, income-present BGs).
    inc = sorted((r for r in rows if r["income"] is not None), key=lambda r: r["income"])
    cum = np.cumsum([r["pop"] for r in inc])
    edges = [np.searchsorted(cum, cum[-1] * k / 5) for k in range(1, 5)]
    quints = []
    lo = 0
    for hi in [*edges, len(inc)]:
        sl = inc[lo:hi]
        quints.append(
            float(np.average([r["A_lived"] for r in sl], weights=[r["pop"] for r in sl]))
        )
        lo = hi

    out = {
        "n_bg": len(rows),
        "n_routed": len(routed),
        "commuter_share_popwtd": c_mean,
        "acs_missing_share_bgs": c_missing / len(rows),
        "gini": {"R_i": g_R, "A_mnl_workers": g_A, "A_lived": g_lived},
        "gini_drop_workers_pct": 100.0 * (1.0 - g_A / g_R),
        "gini_drop_lived_pct": 100.0 * (1.0 - g_lived / g_R),
        "mean_popwtd": {
            "R_i": float(np.average([r["R"] for r in rows], weights=w)),
            "A_lived": float(np.average([r["A_lived"] for r in rows], weights=w)),
        },
        "corr_popwtd": {"A_lived": corrs("A_lived"), "R_i": corrs("R")},
        "A_lived_income_quintiles": quints,
        "note": (
            "A_lived = c*A_mnl + (1-c)*R_i, c = (B08301 workers - WFH)/population per BG. "
            "Non-commuters (children, retirees, unemployed, WFH) are assigned the "
            "residential day; school and non-work travel are out of scope, so A_lived is a "
            "conservative lower bound on non-commuter mobility."
        ),
    }
    with open("data/derived/results/population_mixed.json", "w") as f:
        json.dump(out, f, indent=1)
    print(
        f"population-mixed: commuter share {c_mean:.3f}; Gini R {g_R:.3f} -> "
        f"A_mnl(workers) {g_A:.3f} -> A_lived(everyone) {g_lived:.3f} "
        f"({out['gini_drop_lived_pct']:.1f}% drop)"
    )
    print("wrote data/derived/results/population_mixed.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
