#!/usr/bin/env python3
"""§5c crime-control ladder (mediation reading).

Controlling for crime risks laundering a placement disparity ("cameras go where the crime is").
For each focal demographic we report a ladder of population-weighted regressions of surveillance
exposure on that demographic, adding controls in stages:
  rung 1 — focal demographic only        (TOTAL disparity — the headline)
  rung 2 — + commercial/transit/land-use (jobs, transit access, ambient density)
  rung 3 — + crime density               (the justification channel)
  rung 4 — + 311 public-disorder density (the second justification channel)
The focal coefficient's move from rung 2 -> rung 4 is the share of the (land-use-adjusted)
disparity that runs THROUGH the crime/disorder-justification channel. rung 1 is a headline
number, not a footnote; the no-crime-control estimate is what we lead with.

We run one focal demographic at a time (not all races jointly) so each "total disparity" is the
clean bivariate association and the mediation denominators are well-defined. SEs are HC1-robust but
NOT spatial (the Durbin/Conley spatial inference is §6; here we read coefficient *movement*, which
OLS point estimates identify regardless). Pure-numpy population-weighted WLS; skewed densities are
log1p'd then z-scored; the focal demographic is z-scored so coefficients read as cameras/SD.

SAMPLE (fixed 2026-07: was a silent selection bug). ACS suppresses median household income in
~1,040 small block groups, and those block groups are NOT random -- they skew small-population and
public-housing, i.e. plausibly high-exposure. The previous version required income to be present on
every row, so *every* ladder ran on N=5,547 -- including the %Hispanic/%Black/%White ladders, which
never use income. Each ladder now takes complete cases on ITS OWN specification, so a demographic
ladder keeps the ~940 income-suppressed block groups it never needed to drop. The estimation sample
is held FIXED ACROSS RUNGS within a ladder (complete cases on outcome + focal + all controls), since
the whole point is to read coefficient MOVEMENT across rungs on one sample. N is reported per ladder.

Writes data/derived/results/crime_ladder.json
"""
import csv, json, math, os

import numpy as np

OUTDIR = "data/derived/results"

CONTROLS = ["jobs", "transit", "dens", "stations"]  # rung 2: land use
NEED_BASE = CONTROLS + ["crime", "req311"]  # rungs 3-4 mediators; held fixed across rungs


def load(path, key):
    with open(path) as f:
        return {r[key]: r for r in csv.DictReader(f)}


tab = load("data/derived/exposure/exposure_table_nyc.csv", "GEOID")
cov = load("data/derived/exposure/covariates_bg_nyc.csv", "id")


def fnum(d, k):
    v = d.get(k)
    try:
        return float(v) if v not in (None, "") else None
    except ValueError:
        return None


rows = []
for g in tab:
    if g not in cov:
        continue
    t, c = tab[g], cov[g]
    pop = fnum(t, "population")
    if not pop or pop <= 0:
        continue
    rows.append({
        "pop": pop,
        "R_i": fnum(t, "R_i"), "E_i": fnum(t, "E_i"), "A_mnl": fnum(t, "A_mnl"),
        "pct_black": fnum(t, "pct_black_nh"), "pct_hisp": fnum(t, "pct_hispanic"),
        "pct_white": fnum(t, "pct_white_nh"), "income": fnum(t, "median_hh_income"),
        "jobs": fnum(c, "jobs_wsh"), "crime": fnum(c, "crime_wsh"), "req311": fnum(c, "req311_wsh"),
        "transit": fnum(c, "transit_dist_m"), "dens": fnum(c, "pop_wsh"),
        "stations": fnum(c, "stations_wsh"),
    })


def sample(keys):
    """Complete cases on `keys`, with population weights normalized to mean 1."""
    sub = [r for r in rows if all(r[k] is not None for k in keys)]
    w = np.array([r["pop"] for r in sub], float)
    return sub, w / w.mean()


def wls(y, X, w):
    n = len(y)
    XtW = X.T * w
    beta = np.linalg.solve(XtW @ X, XtW @ y)
    e = y - X @ beta
    bread = np.linalg.inv(XtW @ X)
    meat = (X * (w * e)[:, None]).T @ (X * (w * e)[:, None])
    covb = bread @ meat @ bread * (n / (n - X.shape[1]))
    return beta, np.sqrt(np.diag(covb))


def ladder(yname, focal_key, focal_name, out):
    sub, w = sample([yname, focal_key] + NEED_BASE)
    n = len(sub)

    def col(k, log=False):
        x = np.array([r[k] for r in sub], float)
        return np.log1p(x) if log else x

    def z(x):
        m = np.average(x, weights=w)
        s = math.sqrt(np.average((x - m) ** 2, weights=w))
        return (x - m) / s if s > 0 else x * 0

    y, ones, zf = col(yname), np.ones(n), z(col(focal_key))
    ctrl = [(k, z(col(k, 1))) for k in CONTROLS]
    zcrime, z311 = z(col("crime", 1)), z(col("req311", 1))
    base = [("focal", zf)] + ctrl
    stages = [
        ("1 total (demographic only)", [("focal", zf)]),
        ("2 +land-use", base),
        ("3 +crime", base + [("crime", zcrime)]),
        ("4 +crime+311", base + [("crime", zcrime), ("req311", z311)]),
    ]
    b, line = {}, f"  {focal_name:8s}"
    for name, terms in stages:
        X = np.column_stack([ones] + [t[1] for t in terms])
        beta, se = wls(y, X, w)
        b[name] = (float(beta[1]), float(se[1]))
        line += f" | r{name[0]}: {beta[1]:+.2f}"
    b2, b3 = b["2 +land-use"][0], b["3 +crime"][0]
    b4, se4 = b["4 +crime+311"]
    thru = (b2 - b4) / b2 * 100 if abs(b2) > 1e-6 else float("nan")
    print(line + f"  || land-use-adj {b2:+.2f} -> +crime {b3:+.2f} -> +311 {b4:+.2f}(±{se4:.2f}) "
                 f"[{thru:+.0f}% via crime+311]  N={n}")
    out.setdefault(yname, {})[focal_name] = {
        "n": n,
        "rung1_total": b["1 total (demographic only)"],
        "rung2_landuse": b["2 +land-use"],
        "rung3_crime": b["3 +crime"],
        "rung4_crime_311": b["4 +crime+311"],
        "pct_via_crime_311": thru,
    }


results = {"ladders": {}, "mediator_racialization": {}}
for yname in ["R_i", "E_i", "A_mnl"]:
    sub, w = sample([yname] + NEED_BASE)
    ymean = np.average([r[yname] for r in sub], weights=w)
    print(f"\n########## OUTCOME {yname} (pop-wtd mean {ymean:.1f} cameras); coefs = cameras/SD ##########")
    for fk, fn in [("pct_black", "%Black"), ("pct_hisp", "%Hisp"), ("pct_white", "%White"), ("income", "income")]:
        ladder(yname, fk, fn, results["ladders"])

# Are the mediators themselves racialized? -> the laundering mechanism (both channels)
print("\n########## Are the justification mediators racialized? (mediator ~ demographic, per SD) ##########")
print(f"  {'mediator':16s}{'%Black':>10}{'%Hisp':>10}{'%White':>10}{'income':>10}")
for mkey, mname in [("crime", "log1p(crime)"), ("req311", "log1p(311)")]:
    cells, rec = "", {}
    for fk in ["pct_black", "pct_hisp", "pct_white", "income"]:
        sub, w = sample([mkey, fk])
        x = np.array([r[fk] for r in sub], float)
        m = np.average(x, weights=w)
        s = math.sqrt(np.average((x - m) ** 2, weights=w))
        zx = (x - m) / s if s > 0 else x * 0
        ym = np.log1p(np.array([r[mkey] for r in sub], float))
        beta, se = wls(ym, np.column_stack([np.ones(len(sub)), zx]), w)
        cells += f"{beta[1]:>+10.3f}"
        rec[fk] = [float(beta[1]), float(se[1]), len(sub)]
    results["mediator_racialization"][mname] = rec
    print(f"  {mname:16s}{cells}")

os.makedirs(OUTDIR, exist_ok=True)
with open(os.path.join(OUTDIR, "crime_ladder.json"), "w") as f:
    json.dump(results, f, indent=2)

print("\nSample is per-ladder complete cases (outcome + focal + controls), held fixed across rungs.")
print("Income-focal ladders are necessarily smaller: ACS suppresses income in ~1,040 BGs.")
print("SEs HC1-robust, not spatial (§6 does Conley). Crime = NYPD YTD-2026, 311 = public-disorder;")
print(f"both densities in the 10-min walkshed. wrote {OUTDIR}/crime_ladder.json")
