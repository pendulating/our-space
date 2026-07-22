#!/usr/bin/env python3
"""§6.3 — the equalization, as concentration statistics.

The claim is that surveillance exposure CONVERGES once you route people through a real day:
the residential measure R_i is unequally distributed, but the activity-space measure A_i is
nearly flat. Quintile means show this weakly (the A_mnl income gradient spans ~5 cameras);
concentration statistics show it cleanly, in one number per measure.

Everything here is POPULATION-WEIGHTED, and that distinction is the whole point. A block group
is not a unit of moral concern; a person is. "The most-watched 10% of block groups" is a
statement about geography. "The most-watched 10% of PEOPLE" is a statement about who carries
the burden — and it is the one the paper wants. We report both, because the gap between them
is itself informative (exposure correlates with density, so BG-weighted understates it).

Gini uses the standard trapezoid form on the population-ordered Lorenz curve:
    G = 1 - sum_i p_i (L_{i-1} + L_i)
with p_i the population share of BG i (ascending in exposure) and L_i the cumulative share of
total person-exposure. G = 0 => every person carries identical exposure; G = 1 => one person
carries all of it.

Writes  data/derived/results/inequality_stats.json   (the numbers, for the paper)
        data/derived/results/lorenz_curves.csv       (the curves, for the figure)
"""
import csv, json, os

import numpy as np

TABLE = "data/derived/exposure/exposure_table_nyc.csv"
OUTDIR = "data/derived/results"

# The measures, in the order the argument runs: where you live -> where you go.
MEASURES = [
    ("R_i", "residential (10-min walkshed)"),
    ("A_drive", "activity space, drive routing"),
    ("A_modal", "activity space, observed mode shares"),
    ("A_mnl", "activity space, MNL mode choice"),
    ("E_i", "composite (14h home / 1h commute / 9h work)"),
]


def fnum(d, k):
    v = d.get(k)
    try:
        return float(v) if v not in (None, "") else None
    except ValueError:
        return None


def lorenz(x, w):
    """Population-ordered Lorenz curve. Returns (cum_pop_share, cum_exposure_share)."""
    o = np.argsort(x, kind="mergesort")
    x, w = x[o], w[o]
    p = w / w.sum()
    e = p * x
    if e.sum() <= 0:
        raise ValueError("non-positive total exposure")
    return np.concatenate([[0.0], np.cumsum(p)]), np.concatenate([[0.0], np.cumsum(e) / e.sum()])


def gini(x, w):
    P, L = lorenz(x, w)
    # trapezoid: 1 - sum over bins of (population width) * (mean of the two Lorenz endpoints)
    return float(1.0 - np.sum(np.diff(P) * (L[:-1] + L[1:])))


def wquantile(x, w, q):
    """Population-weighted quantile: the exposure below which share q of PEOPLE fall."""
    o = np.argsort(x, kind="mergesort")
    x, w = x[o], w[o]
    c = (np.cumsum(w) - 0.5 * w) / w.sum()
    return float(np.interp(q, c, x))


def top_share(x, w, frac=0.10):
    """Share of total person-exposure carried by the most-exposed `frac` of PEOPLE."""
    P, L = lorenz(x, w)
    return float(1.0 - np.interp(1.0 - frac, P, L))


def stats(x, w):
    mu = float(np.average(x, weights=w))
    sd = float(np.sqrt(np.average((x - mu) ** 2, weights=w)))
    p10, p50, p90 = (wquantile(x, w, q) for q in (0.10, 0.50, 0.90))
    return {
        "n_bg": int(len(x)),
        "population": float(w.sum()),
        "mean_popwtd": mu,
        "median_popwtd": p50,
        "gini_popwtd": gini(x, w),
        "gini_bgwtd": gini(x, np.ones_like(w)),
        "top_decile_share_popwtd": top_share(x, w),
        "top_decile_share_bgwtd": top_share(x, np.ones_like(w)),
        "p90": p90,
        "p10": p10,
        "p90_p10_ratio": float(p90 / p10) if p10 > 0 else float("inf"),
        "cv": float(sd / mu) if mu > 0 else float("nan"),
    }


rows = []
with open(TABLE) as f:
    for r in csv.DictReader(f):
        pop = fnum(r, "population")
        if not pop or pop <= 0:
            continue
        rec = {"GEOID": r["GEOID"], "pop": pop, "income": fnum(r, "median_hh_income")}
        for k, _ in MEASURES:
            rec[k] = fnum(r, k)
        rows.append(rec)

out = {"source": TABLE, "measures": {}}
print(f"\n{'':34s}{'Gini':>8}{'top-10%':>9}{'P90/P10':>9}{'CV':>7}{'mean':>8}   N")
print(f"{'':34s}{'(pop)':>8}{'of expo':>9}{'':>9}{'':>7}{'':>8}")
print("-" * 84)

for key, label in MEASURES:
    # Zeros are REAL data, not missing: ~30 block groups (~27k residents) genuinely have no
    # camera within a 10-minute walk. Excluding them understated residential inequality and
    # made the mean disagree with the descriptives table (74.3 vs 74.0). Drop only None.
    sub = [r for r in rows if r[key] is not None]
    x = np.array([r[key] for r in sub], float)
    w = np.array([r["pop"] for r in sub], float)
    s = stats(x, w)
    out["measures"][key] = dict(s, label=label)
    print(
        f"  {key:9s} {label:22.22s}{s['gini_popwtd']:8.3f}{s['top_decile_share_popwtd']:9.1%}"
        f"{s['p90_p10_ratio']:9.1f}{s['cv']:7.2f}{s['mean_popwtd']:8.1f}   {s['n_bg']}"
    )

# --- the headline comparison ---------------------------------------------------------------
gr, ga = out["measures"]["R_i"]["gini_popwtd"], out["measures"]["A_mnl"]["gini_popwtd"]
tr, ta = out["measures"]["R_i"]["top_decile_share_popwtd"], out["measures"]["A_mnl"]["top_decile_share_popwtd"]
rr, ra = out["measures"]["R_i"]["p90_p10_ratio"], out["measures"]["A_mnl"]["p90_p10_ratio"]
out["headline"] = {
    "gini_R_i": gr, "gini_A_mnl": ga,
    "gini_drop_pct": 100.0 * (gr - ga) / gr,
    "top_decile_share_R_i": tr, "top_decile_share_A_mnl": ta,
    "p90_p10_R_i": rr, "p90_p10_A_mnl": ra,
}
print("\n" + "=" * 84)
print("THE EQUALIZATION (population-weighted, so these are statements about PEOPLE):")
print(f"  Gini            residential {gr:.3f}  ->  activity space {ga:.3f}   ({(gr-ga)/gr:+.0%})")
print(f"  Top-10% share   residential {tr:.1%}   ->  activity space {ta:.1%}")
print(f"  P90/P10 ratio   residential {rr:.1f}x    ->  activity space {ra:.2f}x")
print("=" * 84)

# --- by income quintile (the §6.3 table) ---------------------------------------------------
# Quintiles of PEOPLE, not of block groups: sort BGs by income, then cut at 20/40/60/80% of
# cumulative population. "The poorest fifth of New Yorkers" is the claim; equal-count BG
# quintiles would instead say "the poorest fifth of block groups", which is a different (and
# for our purposes wrong) statement, since low-income BGs are not equally populous.
inc = sorted([r for r in rows if r["income"] is not None], key=lambda r: r["income"])
cumpop = np.cumsum([r["pop"] for r in inc])
edges = np.searchsorted(cumpop, np.linspace(0, cumpop[-1], 6)[1:-1])
cut = np.split(np.arange(len(inc)), edges)
out["by_income_quintile"] = {}
print("\nBy income quintile (quintiles of POPULATION; population-weighted mean exposure):")
hdr = "  quintile  " + "".join(f"{k:>10}" for k, _ in MEASURES)
print(hdr)
for qi, idx in enumerate(cut, 1):
    grp = [inc[i] for i in idx]
    line, cells = f"  Q{qi}        ", {}
    for key, _ in MEASURES:
        s = [r for r in grp if r[key] is not None]
        if not s:
            line += f"{'-':>10}"
            continue
        v = float(np.average([r[key] for r in s], weights=[r["pop"] for r in s]))
        cells[key] = v
        line += f"{v:10.1f}"
    out["by_income_quintile"][f"Q{qi}"] = dict(
        cells, median_income=float(np.median([r["income"] for r in grp])), n_bg=len(grp)
    )
    print(line)

os.makedirs(OUTDIR, exist_ok=True)
with open(os.path.join(OUTDIR, "inequality_stats.json"), "w") as f:
    json.dump(out, f, indent=2)

# Lorenz curves, thinned to 200 points per measure, for the figure.
with open(os.path.join(OUTDIR, "lorenz_curves.csv"), "w", newline="") as f:
    wtr = csv.writer(f)
    wtr.writerow(["measure", "cum_population_share", "cum_exposure_share"])
    for key, _ in MEASURES:
        sub = [r for r in rows if r[key] is not None]  # zeros are real data (see above)
        P, L = lorenz(np.array([r[key] for r in sub], float), np.array([r["pop"] for r in sub], float))
        for i in np.linspace(0, len(P) - 1, 200).astype(int):
            wtr.writerow([key, f"{P[i]:.6f}", f"{L[i]:.6f}"])

print(f"\nwrote {OUTDIR}/inequality_stats.json + lorenz_curves.csv")
print("Population weights = 2020 decennial BG population (the same weights the instrument uses).")
